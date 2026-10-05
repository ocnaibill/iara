use iara_audio::{Engine, EventSink};
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
