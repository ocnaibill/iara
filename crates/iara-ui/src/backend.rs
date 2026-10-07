//! Ponte com o serviço por D-Bus, em threads próprias, sem polling quando tudo vai bem:
//! - observador: lê o retrato, assina `Changed` e relê a cada aviso; sem serviço, tenta de novo a cada 2 s;
//! - comandos: fila de edições com coalescência (um gesto de slider vira poucas chamadas).
//!
//! As atualizações chegam por `on_update`, chamado nessas threads: quem usa deve levá-las à thread da interface.

use crate::model::{coalesce_ui, UiCommand};
use iara_ipc::{Client, ClientError, OwnerWatch, State, Subscription};
use std::sync::{mpsc, Arc};
use std::time::Duration;

#[derive(Debug)]
pub enum Update {
    State(Box<State>),
    /// O serviço não está no barramento (ou o barramento sumiu).
    Unavailable(String),
    /// O serviço recusou um comando.
    Rejected(String),
    /// Níveis ao vivo por slider (chave em texto, pico linear); só chegam enquanto os medidores estão ligados.
    Levels(Vec<(String, f64)>),
}

pub type OnUpdate = Arc<dyn Fn(Update) + Send + Sync>;

const RETRY: Duration = Duration::from_secs(2);
/// Espera longa por avisos (sem acordar à toa): 1 hora.
const LONG_WAIT: Duration = Duration::from_secs(3600);

pub struct Backend {
    tx: mpsc::Sender<UiCommand>,
}

impl Backend {
    pub fn spawn(bus_name: String, on_update: OnUpdate) -> Self {
        let (tx, rx) = mpsc::channel::<UiCommand>();
        {
            let (name, cb) = (bus_name.clone(), on_update.clone());
            std::thread::spawn(move || watch(&name, &cb));
        }
        std::thread::spawn(move || commands(&bus_name, rx, &on_update));
        Self { tx }
    }

    pub fn send(&self, cmd: UiCommand) {
        let _ = self.tx.send(cmd);
    }
}

fn watch(name: &str, on_update: &OnUpdate) {
    loop {
        let session = Client::connect(name).and_then(|c| {
            let sub = c.subscribe()?;
            let owner = c.watch_owner()?;
            let st = c.state()?;
            Ok((c, sub, owner, st))
        });
        match session {
            Ok((client, sub, owner, first)) => {
                // os níveis têm assinatura própria; o pedido de medidores sai pela conexão dos comandos (ver `commands`)
                if let Ok(levels) = client.subscribe_levels() {
                    let cb = on_update.clone();
                    std::thread::spawn(move || {
                        while let Some(l) = levels.wait(LONG_WAIT) {
                            cb(Update::Levels(l));
                        }
                    });
                }
                on_update(Update::State(Box::new(first)));
                if !follow(&client, sub, owner, on_update) {
                    on_update(Update::Unavailable("o serviço saiu".into()));
                }
            }
            Err(e) => on_update(Update::Unavailable(e.to_string())),
        }
        std::thread::sleep(RETRY);
    }
}

enum Wake {
    Changed,
    /// O serviço saiu do barramento.
    Gone,
}

/// Relê o retrato a cada `Changed`. Devolve `false` quando o serviço some (o que se percebe pelo dono do nome no
/// barramento: um serviço que morre não manda mais `Changed`, e um novo logo depois seria tomado por o mesmo).
fn follow(client: &Client, sub: Subscription, owner: OwnerWatch, on_update: &OnUpdate) -> bool {
    let (tx, rx) = mpsc::channel();
    {
        let tx = tx.clone();
        std::thread::spawn(move || loop {
            match sub.next(LONG_WAIT) {
                Ok(_) => {
                    if tx.send(Wake::Changed).is_err() {
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                // assinatura encerrada (conexão caiu): quem espera descobre ao reler o retrato
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    let _ = tx.send(Wake::Gone);
                    break;
                }
            }
        });
    }
    std::thread::spawn(move || loop {
        match owner.next(LONG_WAIT) {
            Ok(true) | Err(mpsc::RecvTimeoutError::Timeout) => {}
            Ok(false) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = tx.send(Wake::Gone);
                break;
            }
        }
    });
    loop {
        match rx.recv_timeout(LONG_WAIT) {
            Ok(Wake::Gone) | Err(mpsc::RecvTimeoutError::Disconnected) => return false,
            Ok(Wake::Changed) | Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        // coalesce rajadas: um único relido cobre todos os avisos acumulados
        while let Ok(w) = rx.try_recv() {
            if matches!(w, Wake::Gone) {
                return false;
            }
        }
        match client.state() {
            Ok(st) => on_update(Update::State(Box::new(st))),
            Err(ClientError::Unavailable(_)) => return false,
            Err(ClientError::Rejected(m)) => on_update(Update::Rejected(m)),
        }
    }
}

fn commands(name: &str, rx: mpsc::Receiver<UiCommand>, on_update: &OnUpdate) {
    let mut client: Option<Client> = None;
    while let Ok(first) = rx.recv() {
        let mut batch = vec![first];
        batch.extend(rx.try_iter());
        for cmd in coalesce_ui(batch) {
            if client.is_none() {
                client = Client::connect(name).ok();
            }
            let Some(c) = client.as_ref() else {
                on_update(Update::Unavailable("sem barramento de sessão".into()));
                continue;
            };
            let result = match &cmd {
                UiCommand::Edit(e) => c.edit(e),
                UiCommand::Session { key, choice } => c.session_choice(key, choice),
                UiCommand::Deactivate => c.deactivate().map(|()| 0),
                UiCommand::Profile(op) => c.profile_op(op).map(|_| 0),
                UiCommand::Meters(on) => c.set_meters(*on).map(|()| 0),
            };
            match result {
                Ok(_) => {}
                Err(ClientError::Rejected(m)) => on_update(Update::Rejected(m)),
                Err(ClientError::Unavailable(m)) => {
                    client = None;
                    on_update(Update::Unavailable(m));
                }
            }
        }
    }
}
