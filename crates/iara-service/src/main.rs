use iara_audio::{Engine, EventSink};
use iara_service::ipc::ServiceController;
use iara_service::service::{Command, Connector, Msg, Service};
use signal_hook::consts::{SIGINT, SIGTERM};
use std::process::ExitCode;
use std::sync::{mpsc, Arc};

fn main() -> ExitCode {
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
    eprintln!("Iara: serviço encerrado");
    ExitCode::SUCCESS
}
