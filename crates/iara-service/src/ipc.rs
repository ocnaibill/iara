//! Liga o serviço ao IPC: o `Controller` do D-Bus envia mensagens ao laço do serviço e espera a resposta com prazo.
//! O estado continua sendo só do serviço; o IPC nunca o toca diretamente.

use crate::apps::AppView;
use crate::service::{Command, Msg, Snapshot};
use iara_core::apps::Source;
use iara_core::edit::EditCommand;
use iara_ipc::{
    AppEntry, AppSource, Controller, DeviceEntry, ProfileEntry, ProfileOp, SessionChoice, State,
};
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

fn entry(a: AppView) -> AppEntry {
    AppEntry {
        key: a.key,
        display: a.display,
        app_id: a.identity.app_id,
        binary: a.identity.binary,
        name: a.identity.name,
        channel: a.channel,
        source: match a.source {
            Source::SessionOverride => AppSource::Session,
            Source::Rule => AppSource::Rule,
            Source::Default => AppSource::Default,
        },
        state: a.state,
        streams: u32::try_from(a.streams).unwrap_or(u32::MAX),
    }
}

pub fn to_state(s: Snapshot) -> State {
    State {
        apps: s.apps.into_iter().map(entry).collect(),
        default_output: s.status.default_output,
        devices: s
            .status
            .devices
            .into_iter()
            .map(|d| DeviceEntry {
                key: d.name,
                description: d.description,
                output: d.output,
            })
            .collect(),
        profiles: s
            .profiles
            .into_iter()
            .map(|p| ProfileEntry {
                id: p.id,
                name: p.name,
                readable: p.readable,
            })
            .collect(),
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

    fn session_choice(&self, key: String, choice: SessionChoice) -> Result<u64, String> {
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(Msg::Command(Command::SessionChoice {
                key,
                choice,
                reply: Some(reply),
            }))
            .map_err(|_| "serviço encerrando".to_owned())?;
        rx.recv_timeout(REPLY_TIMEOUT)
            .map_err(|_| "o serviço não respondeu a tempo".to_owned())?
    }

    fn profile_op(&self, op: ProfileOp) -> Result<String, String> {
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(Msg::Command(Command::Profile {
                op,
                reply: Some(reply),
            }))
            .map_err(|_| "serviço encerrando".to_owned())?;
        rx.recv_timeout(REPLY_TIMEOUT)
            .map_err(|_| "o serviço não respondeu a tempo".to_owned())?
    }

    fn deactivate(&self) -> Result<(), String> {
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(Msg::Command(Command::Deactivate { reply: Some(reply) }))
            .map_err(|_| "serviço encerrando".to_owned())?;
        rx.recv_timeout(REPLY_TIMEOUT)
            .map_err(|_| "o serviço não respondeu a tempo".to_owned())?
    }

    fn meters(&self, enabled: bool) -> Result<(), String> {
        self.tx
            .send(Msg::Command(Command::Meters(enabled)))
            .map_err(|_| "serviço encerrando".to_owned())
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
