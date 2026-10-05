//! Liga o serviço ao IPC: o `Controller` do D-Bus envia mensagens ao laço do serviço e espera a resposta com prazo.
//! O estado continua sendo só do serviço; o IPC nunca o toca diretamente.

use crate::service::{Command, Msg, Snapshot};
use iara_core::edit::EditCommand;
use iara_ipc::{Controller, State};
use std::sync::mpsc;
use std::time::Duration;

/// Prazo para o laço do serviço responder (ele pode estar aplicando o plano ao PipeWire, até 5 s no pior caso).
const REPLY_TIMEOUT: Duration = Duration::from_secs(8);

pub struct ServiceController {
    tx: mpsc::Sender<Msg>,
}

impl ServiceController {
    pub fn new(tx: mpsc::Sender<Msg>) -> Self {
        Self { tx }
    }
}

pub fn to_state(s: Snapshot) -> State {
    State {
        serial: s.serial,
        profile: s.profile,
        connected: s.status.connected,
        persist_error: s.status.persist_error,
        absent_devices: s.status.absent_devices,
        reconnect_attempts: s.status.reconnect_attempts,
    }
}

impl Controller for ServiceController {
    fn state(&self) -> Result<State, String> {
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(Msg::Command(Command::GetState { reply }))
            .map_err(|_| "serviço encerrando".to_owned())?;
        rx.recv_timeout(REPLY_TIMEOUT)
            .map(to_state)
            .map_err(|_| "o serviço não respondeu a tempo".to_owned())
    }

    fn edit(&self, cmd: EditCommand) -> Result<u64, String> {
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(Msg::Command(Command::Edit {
                cmd,
                reply: Some(reply),
            }))
            .map_err(|_| "serviço encerrando".to_owned())?;
        rx.recv_timeout(REPLY_TIMEOUT)
            .map_err(|_| "o serviço não respondeu a tempo".to_owned())?
    }
}
