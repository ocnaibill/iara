use crate::{Controller, State, INTERFACE, OBJECT_PATH};
use iara_core::edit::{EditCommand, MicSend, SendKind};
use iara_core::Gain;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use zbus::blocking::connection::Builder;
use zbus::blocking::Connection;
use zbus::fdo;

struct Mixer {
    controller: Arc<dyn Controller>,
    leases: Arc<MeterLeases>,
}

/// Pedidos de medidores, um por cliente do barramento (nome único da conexão). O controlador só é avisado quando o conjunto
/// passa de vazio a não vazio e vice-versa; um cliente que some do barramento perde o pedido sozinho.
struct MeterLeases {
    holders: Mutex<HashSet<String>>,
    controller: Arc<dyn Controller>,
}

impl MeterLeases {
    fn set(&self, who: &str, on: bool) -> Result<(), String> {
        let mut h = self.holders.lock().unwrap_or_else(|e| e.into_inner());
        let before = !h.is_empty();
        if on {
            h.insert(who.to_owned());
        } else {
            h.remove(who);
        }
        let after = !h.is_empty();
        if before != after {
            self.controller.meters(after)?;
        }
        Ok(())
    }
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

pub(crate) fn state_to_tuple(s: &State) -> Result<crate::StateWire, String> {
    let toml = iara_store::profile_to_toml(&s.profile).map_err(|e| e.to_string())?;
    Ok((
        s.serial,
        toml,
        s.connected,
        s.persist_error.clone().unwrap_or_default(),
        s.absent_devices.clone(),
        s.reconnect_attempts,
        crate::apps_to_toml(&s.apps)?,
        s.default_output.as_str().to_owned(),
        s.profiles
            .iter()
            .map(|p| (p.id.clone(), p.name.clone(), p.readable))
            .collect(),
        s.devices
            .iter()
            .map(|d| (d.key.clone(), d.description.clone(), d.output))
            .collect(),
    ))
}

#[zbus::interface(name = "dev.iara.Mixer1")]
impl Mixer {
    /// (versão, perfil em TOML, conectado ao áudio, erro de gravação ("" = nenhum), dispositivos ausentes, tentativas de reconexão,
    /// aplicativos em TOML, saída padrão: disabled|waiting|active|released, perfis (id, nome, legível))
    fn get_state(&self) -> fdo::Result<crate::StateWire> {
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

    /// Escolha só desta sessão para um aplicativo (chave de identidade): `channel` vazio = Não atribuídos. Não altera o perfil.
    fn set_app_session(&self, key: String, channel: String) -> fdo::Result<u64> {
        let choice = match opt(channel) {
            Some(id) => crate::SessionChoice::Channel(id),
            None => crate::SessionChoice::Unassigned,
        };
        self.controller
            .session_choice(key, choice)
            .map_err(fdo::Error::InvalidArgs)
    }

    /// Apaga a escolha temporária e reaplica a regra do perfil.
    fn clear_app_session(&self, key: String) -> fdo::Result<u64> {
        self.controller
            .session_choice(key, crate::SessionChoice::Clear)
            .map_err(fdo::Error::InvalidArgs)
    }

    /// Troca o perfil ativo.
    fn switch_profile(&self, id: String) -> fdo::Result<String> {
        self.controller
            .profile_op(crate::ProfileOp::Switch(id))
            .map_err(fdo::Error::InvalidArgs)
    }

    /// Cria um perfil novo (herda os dispositivos do ativo) e devolve o id; não troca para ele.
    fn create_profile(&self, name: String) -> fdo::Result<String> {
        self.controller
            .profile_op(crate::ProfileOp::Create(name))
            .map_err(fdo::Error::InvalidArgs)
    }

    /// Cópia independente de um perfil; devolve o id novo.
    fn duplicate_profile(&self, id: String, name: String) -> fdo::Result<String> {
        self.controller
            .profile_op(crate::ProfileOp::Duplicate { id, name })
            .map_err(fdo::Error::InvalidArgs)
    }

    fn rename_profile(&self, id: String, name: String) -> fdo::Result<String> {
        self.controller
            .profile_op(crate::ProfileOp::Rename { id, name })
            .map_err(fdo::Error::InvalidArgs)
    }

    /// Manda o perfil para a lixeira (recuperável); o ativo e o último perfil não podem ser excluídos.
    fn delete_profile(&self, id: String) -> fdo::Result<String> {
        self.controller
            .profile_op(crate::ProfileOp::Delete(id))
            .map_err(fdo::Error::InvalidArgs)
    }

    /// “Desligar mixer / voltar ao áudio normal”: restaura a saída padrão anterior (se ainda for a do Iara) e encerra o serviço.
    fn deactivate(&self) -> fdo::Result<()> {
        self.controller.deactivate().map_err(fdo::Error::Failed)
    }

    /// Liga/desliga os medidores ao vivo para quem chama (um pedido por conexão; some se a conexão cair). Use a mesma
    /// conexão para ligar e desligar.
    fn set_meters(
        &self,
        enabled: bool,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> fdo::Result<()> {
        let who = header
            .sender()
            .map(|s| s.to_string())
            .ok_or_else(|| fdo::Error::InvalidArgs("chamada sem remetente".into()))?;
        self.leases.set(&who, enabled).map_err(fdo::Error::Failed)
    }

    /// O retrato mudou (perfil ou status); releia com `GetState`.
    #[zbus(signal)]
    async fn changed(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        serial: u64,
    ) -> zbus::Result<()>;

    /// Pico linear por slider (chave em texto, ver `iara_core::meter::MeterKey::encode`), cerca de 20 por segundo, só
    /// enquanto algum cliente pediu medidores.
    #[zbus(signal)]
    async fn levels(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        levels: Vec<(String, f64)>,
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

    /// Função para o serviço entregar os níveis por slider: emite `Levels`.
    pub fn levels_notifier(&self) -> crate::LevelsNotifier {
        let conn = self.connection.clone();
        Box::new(move |levels| {
            // Falha de envio (barramento caiu) é ignorada: os níveis são efêmeros e o próximo substitui este.
            let _ = conn.emit_signal(None::<&str>, OBJECT_PATH, INTERFACE, "Levels", &(levels,));
        })
    }

    pub fn connection(&self) -> &Connection {
        &self.connection
    }
}

/// Publica a interface no barramento de sessão sob `name`. Falha se o nome já tiver dono (instância única).
pub fn serve(name: &str, controller: Arc<dyn Controller>) -> zbus::Result<Server> {
    let leases = Arc::new(MeterLeases {
        holders: Mutex::new(HashSet::new()),
        controller: controller.clone(),
    });
    // Instância única: não permite que outra tome o nome (allow_name_replacements) nem toma o de outra (replace_existing_names).
    let connection = Builder::session()?
        .allow_name_replacements(false)
        .replace_existing_names(false)
        .name(name.to_owned())?
        .serve_at(
            OBJECT_PATH,
            Mixer {
                controller,
                leases: leases.clone(),
            },
        )?
        .build()?;
    watch_clients(&connection, leases)?;
    Ok(Server { connection })
}

/// Solta o pedido de medidores de um cliente que saiu do barramento (processo encerrado sem desligar).
fn watch_clients(connection: &Connection, leases: Arc<MeterLeases>) -> zbus::Result<()> {
    let proxy = zbus::blocking::fdo::DBusProxy::new(connection)?;
    let changes = proxy.receive_name_owner_changed()?;
    std::thread::Builder::new()
        .name("iara-ipc-clients".into())
        .spawn(move || {
            for change in changes {
                let Ok(args) = change.args() else { continue };
                if args.new_owner().is_none() {
                    let _ = leases.set(args.name().as_str(), false);
                }
            }
        })
        .map_err(|e| zbus::Error::Failure(e.to_string()))?;
    Ok(())
}
