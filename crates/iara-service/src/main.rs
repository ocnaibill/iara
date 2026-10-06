use iara_audio::{Engine, EventSink};
use iara_service::ipc::ServiceController;
use iara_service::service::{Command, Connector, Msg, Service};
use signal_hook::consts::{SIGINT, SIGTERM};
use std::process::ExitCode;
use std::sync::{mpsc, Arc};

/// Recuperação: se um serviço anterior caiu deixando o Iara como saída padrão do sistema, devolve o padrão anterior
/// (só se o padrão atual ainda for o do Iara) sem subir o mixer. Seguro de rodar a qualquer momento.
fn restore_default() -> ExitCode {
    use iara_audio::Event;
    use iara_service::default_sink::INSTALLED;
    let store = match iara_store::Store::from_xdg() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Iara: {e}");
            return ExitCode::FAILURE;
        }
    };
    let state = match store.load_default_sink_state() {
        Ok(Some(s)) => s,
        Ok(None) => {
            eprintln!("Iara: não há saída padrão anterior registrada; nada a restaurar.");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("Iara: registro da saída padrão ilegível: {e}");
            return ExitCode::FAILURE;
        }
    };
    let (tx, rx) = mpsc::channel();
    let sink: EventSink = Arc::new(move |e| {
        if let Event::DefaultSink { configured, .. } = e {
            let _ = tx.send(configured);
        }
    });
    let engine = match Engine::start_with(sink) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Iara: sem acesso ao PipeWire: {e}");
            return ExitCode::FAILURE;
        }
    };
    let current = rx
        .recv_timeout(std::time::Duration::from_secs(3))
        .ok()
        .flatten();
    if current.as_deref() != Some(INSTALLED) {
        eprintln!(
            "Iara: a saída padrão atual ({}) não é a do Iara; nada alterado.",
            current.as_deref().unwrap_or("nenhuma")
        );
    } else {
        let previous = state.previous.filter(|p| p != INSTALLED);
        eprintln!(
            "Iara: restaurando a saída padrão para {}",
            previous.as_deref().unwrap_or("(escolha automática)")
        );
        if engine.set_default_sink(previous).is_err() {
            return ExitCode::FAILURE;
        }
    }
    drop(engine); // o comando entra na fila antes do encerramento do motor
    let _ = store.clear_default_sink_state();
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        Some("--restore-default") => return restore_default(),
        Some("--help" | "-h") => {
            eprintln!("uso: iara-service [--restore-default]\n  --restore-default: devolve a saída padrão anterior se um serviço anterior caiu deixando o Iara como padrão\n  IARA_BUS_NAME: nome no D-Bus (testes)");
            return ExitCode::SUCCESS;
        }
        _ => {}
    }
    let store = match iara_store::Store::from_xdg() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Iara: {e}");
            return ExitCode::FAILURE;
        }
    };
    let (tx, rx) = mpsc::channel::<Msg>();
    let engine_tx = tx.clone();
    let sink: EventSink = Arc::new(move |e| {
        let _ = engine_tx.send(Msg::Engine(e));
    });
    let connect: Connector<Engine> = Box::new(Engine::start_with);
    let mut service = match Service::new(store, connect, sink) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Iara: não foi possível carregar o perfil: {e}");
            return ExitCode::FAILURE;
        }
    };
    // IPC: sem barramento de sessão o serviço segue headless; nome já ocupado = outra instância rodando.
    // `IARA_BUS_NAME` existe para testes e desenvolvimento (outra instância sem colidir com a real).
    let bus_name = std::env::var("IARA_BUS_NAME").unwrap_or_else(|_| iara_ipc::BUS_NAME.to_owned());
    let _ipc = match iara_ipc::serve(&bus_name, Arc::new(ServiceController::new(tx.clone()))) {
        Ok(server) => {
            service.set_notifier(server.notifier());
            service.set_levels_notifier(server.levels_notifier());
            Some(server)
        }
        Err(zbus::Error::NameTaken) => {
            eprintln!("Iara: já existe uma instância do serviço rodando");
            return ExitCode::FAILURE;
        }
        Err(e) => {
            eprintln!("Iara: sem IPC ({e}); o serviço segue sem interface");
            None
        }
    };
    match signal_hook::iterator::Signals::new([SIGINT, SIGTERM]) {
        Ok(mut signals) => {
            let tx = tx.clone();
            std::thread::spawn(move || {
                if signals.forever().next().is_some() {
                    let _ = tx.send(Msg::Command(Command::Shutdown));
                }
            });
        }
        Err(e) => {
            eprintln!("Iara: sem tratamento de sinais ({e}); encerre pelo gerenciador de serviço")
        }
    }
    eprintln!("Iara: serviço iniciado (perfil {})", service.profile().id);
    service.run(&rx);
    // dá tempo de a resposta de `Deactivate` (e as demais pendentes) chegar ao cliente antes de largar o barramento
    std::thread::sleep(std::time::Duration::from_millis(300));
    eprintln!("Iara: serviço encerrado");
    ExitCode::SUCCESS
}
