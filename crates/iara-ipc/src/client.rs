use crate::{State, INTERFACE, OBJECT_PATH};
use iara_core::edit::{EditCommand, MicSend, SendKind};
use iara_core::Gain;
use std::time::Duration;
use zbus::blocking::{Connection, Proxy};

#[derive(Debug)]
pub enum ClientError {
    /// Serviço ausente no barramento (ou barramento indisponível).
    Unavailable(String),
    /// O serviço recusou o comando (argumento inválido) ou falhou.
    Rejected(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(m) => write!(f, "serviço indisponível: {m}"),
            Self::Rejected(m) => write!(f, "comando recusado: {m}"),
        }
    }
}

impl std::error::Error for ClientError {}

/// Erros do próprio barramento que significam "o serviço não está lá", não "o comando foi recusado".
fn is_unavailable(name: &str) -> bool {
    matches!(
        name,
        "org.freedesktop.DBus.Error.ServiceUnknown"
            | "org.freedesktop.DBus.Error.NameHasNoOwner"
            | "org.freedesktop.DBus.Error.NoReply"
            | "org.freedesktop.DBus.Error.Disconnected"
            | "org.freedesktop.DBus.Error.UnknownObject"
    )
}

impl From<zbus::Error> for ClientError {
    fn from(e: zbus::Error) -> Self {
        match &e {
            zbus::Error::MethodError(name, msg, _) => {
                if is_unavailable(name.as_str()) {
                    Self::Unavailable(msg.clone().unwrap_or_else(|| name.to_string()))
                } else {
                    Self::Rejected(msg.clone().unwrap_or_else(|| name.to_string()))
                }
            }
            zbus::Error::FDO(f) => match **f {
                zbus::fdo::Error::ServiceUnknown(_)
                | zbus::fdo::Error::NameHasNoOwner(_)
                | zbus::fdo::Error::NoReply(_)
                | zbus::fdo::Error::Disconnected(_)
                | zbus::fdo::Error::UnknownObject(_) => Self::Unavailable(e.to_string()),
                _ => Self::Rejected(e.to_string()),
            },
            _ => Self::Unavailable(e.to_string()),
        }
    }
}

pub struct Client {
    proxy: Proxy<'static>,
}

fn kind(s: SendKind) -> &'static str {
    match s {
        SendKind::Personal => "personal",
        SendKind::Transmission => "transmission",
    }
}

fn mic_kind(s: MicSend) -> &'static str {
    match s {
        MicSend::Applications => "applications",
        MicSend::Personal => "personal",
        MicSend::Transmission => "transmission",
    }
}

/// Silêncio vai como `-inf`, o mesmo contrato do servidor.
fn db(g: Gain) -> f64 {
    g.db().unwrap_or(f64::NEG_INFINITY)
}

impl Client {
    pub fn connect(name: impl Into<String>) -> Result<Self, ClientError> {
        let connection = Connection::session()?;
        let proxy = Proxy::new(&connection, name.into(), OBJECT_PATH, INTERFACE)?;
        Ok(Self { proxy })
    }

    pub fn state(&self) -> Result<State, ClientError> {
        let (
            serial,
            toml,
            connected,
            persist_error,
            absent_devices,
            reconnect_attempts,
            apps,
            default_output,
        ): (u64, String, bool, String, Vec<String>, u32, String, String) =
            self.proxy.call("GetState", &())?;
        let apps = crate::apps_from_toml(&apps).map_err(ClientError::Rejected)?;
        let profile = iara_store::profile_from_toml(&toml)
            .map_err(|e| ClientError::Rejected(e.to_string()))?;
        Ok(State {
            serial,
            profile,
            connected,
            persist_error: (!persist_error.is_empty()).then_some(persist_error),
            absent_devices,
            reconnect_attempts,
            apps,
            default_output: crate::DefaultOutput::parse(&default_output).unwrap_or_default(),
        })
    }

    /// Envia a edição; devolve a nova versão do estado.
    pub fn edit(&self, cmd: &EditCommand) -> Result<u64, ClientError> {
        use EditCommand as E;
        let p = &self.proxy;
        let serial = match cmd {
            E::SetChannelGain {
                channel,
                send,
                gain,
            } => p.call("SetChannelGain", &(channel, kind(*send), db(*gain)))?,
            E::SetChannelMute {
                channel,
                send,
                muted,
            } => p.call("SetChannelMute", &(channel, kind(*send), *muted))?,
            E::SetChannelEnabled {
                channel,
                send,
                enabled,
            } => p.call("SetChannelEnabled", &(channel, kind(*send), *enabled))?,
            E::SetMasterGain { send, gain } => {
                p.call("SetMasterGain", &(kind(*send), db(*gain)))?
            }
            E::SetMasterMute { send, muted } => p.call("SetMasterMute", &(kind(*send), *muted))?,
            E::SetMicGlobalMute(m) => p.call("SetMicGlobalMute", &(*m,))?,
            E::SetMicInputGain(g) => p.call("SetMicInputGain", &(db(*g),))?,
            E::SetMicSendGain { send, gain } => {
                p.call("SetMicSendGain", &(mic_kind(*send), db(*gain)))?
            }
            E::SetMicSendMute { send, muted } => {
                p.call("SetMicSendMute", &(mic_kind(*send), *muted))?
            }
            E::SetMicSendEnabled { send, enabled } => {
                p.call("SetMicSendEnabled", &(mic_kind(*send), *enabled))?
            }
            E::SetChatMixPosition(x) => p.call("SetChatMixPosition", &(*x,))?,
            E::SetChatMixChannels(pair) => {
                let (a, b) = pair.clone().unwrap_or_default();
                p.call("SetChatMixChannels", &(a, b))?
            }
            E::SetPreferredOutput(d) => {
                p.call("SetPreferredOutput", &(d.clone().unwrap_or_default(),))?
            }
            E::SetPreferredMicrophone(d) => {
                p.call("SetPreferredMicrophone", &(d.clone().unwrap_or_default(),))?
            }
            E::AddChannel { id, name } => p.call("AddChannel", &(id, name))?,
            E::RenameChannel { channel, name } => p.call("RenameChannel", &(channel, name))?,
            E::RemoveChannel {
                channel,
                destination,
            } => p.call(
                "RemoveChannel",
                &(channel, destination.clone().unwrap_or_default()),
            )?,
            E::AssignApp { matcher, channel } => p.call(
                "AssignApp",
                &(
                    matcher.app_id.clone().unwrap_or_default(),
                    matcher.binary.clone().unwrap_or_default(),
                    matcher.name.clone().unwrap_or_default(),
                    channel.clone().unwrap_or_default(),
                ),
            )?,
        };
        Ok(serial)
    }

    /// Escolha só desta sessão para um aplicativo; devolve a nova versão do estado.
    pub fn session_choice(
        &self,
        key: &str,
        choice: &crate::SessionChoice,
    ) -> Result<u64, ClientError> {
        let p = &self.proxy;
        Ok(match choice {
            crate::SessionChoice::Channel(id) => p.call("SetAppSession", &(key, id.as_str()))?,
            crate::SessionChoice::Unassigned => p.call("SetAppSession", &(key, ""))?,
            crate::SessionChoice::Clear => p.call("ClearAppSession", &(key,))?,
        })
    }

    /// “Desligar mixer / voltar ao áudio normal”: o serviço restaura a saída padrão anterior e encerra.
    pub fn deactivate(&self) -> Result<(), ClientError> {
        match self.proxy.call::<_, _, ()>("Deactivate", &()) {
            Ok(()) => Ok(()),
            // o serviço encerra logo após responder: se ele sumiu antes de a resposta chegar, o resultado desejado foi alcançado
            Err(e) => match ClientError::from(e) {
                ClientError::Unavailable(_) => Ok(()),
                other => Err(other),
            },
        }
    }

    /// Assina o sinal `Changed`. Crie a assinatura ANTES de agir se não puder perder avisos; ela vive até o fim do processo
    /// (uma por aplicação basta: guarde-a e releia o estado a cada aviso).
    pub fn subscribe(&self) -> Result<Subscription, ClientError> {
        let signals = self.proxy.receive_signal("Changed")?;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for msg in signals {
                if let Ok(serial) = msg.body().deserialize::<u64>() {
                    if tx.send(serial).is_err() {
                        break;
                    }
                }
            }
        });
        Ok(Subscription { rx })
    }
}

/// Avisos `Changed(serial)` recebidos do serviço.
pub struct Subscription {
    rx: std::sync::mpsc::Receiver<u64>,
}

impl Subscription {
    /// Próxima versão anunciada, ou `None` se nada chegou no prazo.
    pub fn wait(&self, timeout: Duration) -> Option<u64> {
        self.rx.recv_timeout(timeout).ok()
    }

    /// Descarta o acumulado e devolve a versão mais recente (coalescendo rajadas), se houver.
    pub fn latest(&self) -> Option<u64> {
        self.rx.try_iter().last()
    }
}
