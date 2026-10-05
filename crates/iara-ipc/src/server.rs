use crate::{Controller, State, INTERFACE, OBJECT_PATH};
use iara_core::edit::{EditCommand, MicSend, SendKind};
use iara_core::Gain;
use std::sync::Arc;
use zbus::blocking::connection::Builder;
use zbus::blocking::Connection;
use zbus::fdo;

struct Mixer {
    controller: Arc<dyn Controller>,
}

fn send(s: &str) -> fdo::Result<SendKind> {
    match s {
        "personal" => Ok(SendKind::Personal),
        "transmission" => Ok(SendKind::Transmission),
        _ => Err(fdo::Error::InvalidArgs(format!(
            "envio inválido: {s:?} (use personal ou transmission)"
        ))),
    }
}

fn mic_send(s: &str) -> fdo::Result<MicSend> {
    match s {
        "applications" => Ok(MicSend::Applications),
        "personal" => Ok(MicSend::Personal),
        "transmission" => Ok(MicSend::Transmission),
        _ => Err(fdo::Error::InvalidArgs(format!(
            "envio do microfone inválido: {s:?} (use applications, personal ou transmission)"
        ))),
    }
}

fn gain(db: f64) -> fdo::Result<Gain> {
    Gain::from_db_or_silence(db).map_err(|m| fdo::Error::InvalidArgs(m.to_owned()))
}

fn opt(s: String) -> Option<String> {
    (!s.is_empty()).then_some(s)
}

impl Mixer {
    fn run(&self, cmd: EditCommand) -> fdo::Result<u64> {
        self.controller.edit(cmd).map_err(fdo::Error::InvalidArgs)
    }
}

type StateTuple = (u64, String, bool, String, Vec<String>, u32);

pub(crate) fn state_to_tuple(s: &State) -> Result<StateTuple, String> {
    let toml = iara_store::profile_to_toml(&s.profile).map_err(|e| e.to_string())?;
    Ok((
        s.serial,
        toml,
        s.connected,
        s.persist_error.clone().unwrap_or_default(),
        s.absent_devices.clone(),
        s.reconnect_attempts,
    ))
}

#[zbus::interface(name = "dev.iara.Mixer1")]
impl Mixer {
    /// (versão, perfil em TOML, conectado ao áudio, erro de gravação ("" = nenhum), dispositivos ausentes, tentativas de reconexão)
    fn get_state(&self) -> fdo::Result<StateTuple> {
        let s = self.controller.state().map_err(fdo::Error::Failed)?;
        state_to_tuple(&s).map_err(fdo::Error::Failed)
    }

    fn set_channel_gain(&self, channel: String, send_kind: String, db: f64) -> fdo::Result<u64> {
        self.run(EditCommand::SetChannelGain {
            channel,
            send: send(&send_kind)?,
            gain: gain(db)?,
        })
    }

    fn set_channel_mute(
        &self,
        channel: String,
        send_kind: String,
        muted: bool,
    ) -> fdo::Result<u64> {
        self.run(EditCommand::SetChannelMute {
            channel,
            send: send(&send_kind)?,
            muted,
        })
    }

    fn set_channel_enabled(
        &self,
        channel: String,
        send_kind: String,
        enabled: bool,
    ) -> fdo::Result<u64> {
        self.run(EditCommand::SetChannelEnabled {
            channel,
            send: send(&send_kind)?,
            enabled,
        })
    }

    fn set_master_gain(&self, send_kind: String, db: f64) -> fdo::Result<u64> {
        self.run(EditCommand::SetMasterGain {
            send: send(&send_kind)?,
            gain: gain(db)?,
        })
    }

    fn set_master_mute(&self, send_kind: String, muted: bool) -> fdo::Result<u64> {
        self.run(EditCommand::SetMasterMute {
            send: send(&send_kind)?,
            muted,
        })
    }

    fn set_mic_global_mute(&self, muted: bool) -> fdo::Result<u64> {
        self.run(EditCommand::SetMicGlobalMute(muted))
    }

    fn set_mic_input_gain(&self, db: f64) -> fdo::Result<u64> {
        self.run(EditCommand::SetMicInputGain(gain(db)?))
    }

    fn set_mic_send_gain(&self, send_kind: String, db: f64) -> fdo::Result<u64> {
        self.run(EditCommand::SetMicSendGain {
            send: mic_send(&send_kind)?,
            gain: gain(db)?,
        })
    }

    fn set_mic_send_mute(&self, send_kind: String, muted: bool) -> fdo::Result<u64> {
        self.run(EditCommand::SetMicSendMute {
            send: mic_send(&send_kind)?,
            muted,
        })
    }

    fn set_mic_send_enabled(&self, send_kind: String, enabled: bool) -> fdo::Result<u64> {
        self.run(EditCommand::SetMicSendEnabled {
            send: mic_send(&send_kind)?,
            enabled,
        })
    }

    fn set_chat_mix_position(&self, position: f64) -> fdo::Result<u64> {
        self.run(EditCommand::SetChatMixPosition(position))
    }

    /// Strings vazias desativam o ChatMix.
    fn set_chat_mix_channels(&self, first: String, second: String) -> fdo::Result<u64> {
        let pair = match (opt(first), opt(second)) {
            (Some(a), Some(b)) => Some((a, b)),
            (None, None) => None,
            _ => {
                return Err(fdo::Error::InvalidArgs(
                    "informe os dois canais ou nenhum".into(),
                ))
            }
        };
        self.run(EditCommand::SetChatMixChannels(pair))
    }

    /// String vazia limpa a preferência.
    fn set_preferred_output(&self, device: String) -> fdo::Result<u64> {
        self.run(EditCommand::SetPreferredOutput(opt(device)))
    }

    fn set_preferred_microphone(&self, device: String) -> fdo::Result<u64> {
        self.run(EditCommand::SetPreferredMicrophone(opt(device)))
    }

    fn add_channel(&self, id: String, name: String) -> fdo::Result<u64> {
        self.run(EditCommand::AddChannel { id, name })
    }

    fn rename_channel(&self, channel: String, name: String) -> fdo::Result<u64> {
        self.run(EditCommand::RenameChannel { channel, name })
    }

    /// `destination` é o canal que herda as regras do removido; vazio = Não atribuídos (as regras são apagadas).
    fn remove_channel(&self, channel: String, destination: String) -> fdo::Result<u64> {
        self.run(EditCommand::RemoveChannel {
            channel,
            destination: opt(destination),
        })
    }

    /// Associa um aplicativo a um canal salvando a regra no perfil. Os campos de identidade vazios são ignorados (ao menos
    /// um é obrigatório); `channel` vazio apaga a regra (o aplicativo volta a Não atribuídos).
    fn assign_app(
        &self,
        app_id: String,
        binary: String,
        name: String,
        channel: String,
    ) -> fdo::Result<u64> {
        self.run(EditCommand::AssignApp {
            matcher: iara_core::apps::AppMatcher {
                app_id: opt(app_id),
                binary: opt(binary),
                name: opt(name),
            },
            channel: opt(channel),
        })
    }

    /// O retrato mudou (perfil ou status); releia com `GetState`.
    #[zbus(signal)]
    async fn changed(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        serial: u64,
    ) -> zbus::Result<()>;
}

/// Servidor ativo; ao ser descartado, solta o nome no barramento.
pub struct Server {
    connection: Connection,
}

impl Server {
    /// Função para o serviço registrar como notificador: emite `Changed(serial)`.
    pub fn notifier(&self) -> crate::Notifier {
        let conn = self.connection.clone();
        Box::new(move |serial| {
            if let Err(e) =
                conn.emit_signal(None::<&str>, OBJECT_PATH, INTERFACE, "Changed", &(serial,))
            {
                eprintln!("iara: falha ao emitir Changed: {e}");
            }
        })
    }

    pub fn connection(&self) -> &Connection {
        &self.connection
    }
}

/// Publica a interface no barramento de sessão sob `name`. Falha se o nome já tiver dono (instância única).
pub fn serve(name: &str, controller: Arc<dyn Controller>) -> zbus::Result<Server> {
    // Instância única: não permite que outra tome o nome (allow_name_replacements) nem toma o de outra (replace_existing_names).
    let connection = Builder::session()?
        .allow_name_replacements(false)
        .replace_existing_names(false)
        .name(name.to_owned())?
        .serve_at(OBJECT_PATH, Mixer { controller })?
        .build()?;
    Ok(Server { connection })
}
