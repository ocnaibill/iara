//! Comandos de edição do perfil: a única forma de a interface (ou qualquer cliente) mudar o estado do serviço.
//! Funcional e atômico: `apply` devolve um perfil novo já validado, ou erro sem efeito. A classe do comando
//! (contínuo ou estrutural) diz como ele entra no histórico (spec 8.14).

use crate::apps::{AppMatcher, Rule};
use crate::topology::plan;
use crate::{
    chatmix, is_valid_id, is_valid_text, Channel, DevicePreference, Gain, Profile, SendControl,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendKind {
    Personal,
    Transmission,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MicSend {
    Applications,
    Personal,
    Transmission,
}

#[derive(Clone, Debug, PartialEq)]
pub enum EditCommand {
    SetChannelGain {
        channel: String,
        send: SendKind,
        gain: Gain,
    },
    SetChannelMute {
        channel: String,
        send: SendKind,
        muted: bool,
    },
    SetChannelEnabled {
        channel: String,
        send: SendKind,
        enabled: bool,
    },
    SetMasterGain {
        send: SendKind,
        gain: Gain,
    },
    SetMasterMute {
        send: SendKind,
        muted: bool,
    },
    SetMicGlobalMute(bool),
    SetMicInputGain(Gain),
    SetMicSendGain {
        send: MicSend,
        gain: Gain,
    },
    SetMicSendMute {
        send: MicSend,
        muted: bool,
    },
    SetMicSendEnabled {
        send: MicSend,
        enabled: bool,
    },
    SetChatMixPosition(f64),
    SetChatMixChannels(Option<(String, String)>),
    /// `None` limpa a preferência (sem dispositivo).
    SetPreferredOutput(Option<String>),
    SetPreferredMicrophone(Option<String>),
    AddChannel {
        id: String,
        name: String,
    },
    RenameChannel {
        channel: String,
        name: String,
    },
    /// Remove o canal; as regras que apontavam para ele vão para `destination` (outro canal) ou, com `None`, são
    /// apagadas (os aplicativos voltam a Não atribuídos). Nunca assume um destino por conta própria (spec 8.3).
    RemoveChannel {
        channel: String,
        destination: Option<String>,
    },
    /// Associa o aplicativo identificado por `matcher` a um canal (cria ou troca a regra de mesmo `matcher`);
    /// `None` apaga a regra (o aplicativo volta a Não atribuídos).
    AssignApp {
        matcher: AppMatcher,
        channel: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditClass {
    /// Ajuste de nível, mute ou habilitação: agrupado no histórico.
    Continuous,
    /// Criar/remover/renomear canal, trocar dispositivo ou par do ChatMix: revisão própria.
    Structural,
}

#[derive(Debug, PartialEq, Eq)]
pub enum EditError {
    UnknownChannel(String),
    DuplicateChannel(String),
    InvalidId(String),
    InvalidText,
    InvalidValue(&'static str),
    /// O resultado não passaria na validação do plano (referências quebradas etc.).
    Invalid(String),
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownChannel(c) => write!(f, "canal desconhecido: {c}"),
            Self::DuplicateChannel(c) => write!(f, "já existe um canal com o id {c}"),
            Self::InvalidId(c) => write!(f, "id inválido: {c:?}"),
            Self::InvalidText => write!(
                f,
                "texto inválido (vazio, longo demais ou com caracteres de controle)"
            ),
            Self::InvalidValue(m) => write!(f, "{m}"),
            Self::Invalid(m) => write!(f, "perfil resultante inválido: {m}"),
        }
    }
}

impl std::error::Error for EditError {}

fn channel<'a>(p: &'a mut Profile, id: &str) -> Result<&'a mut Channel, EditError> {
    p.channels
        .iter_mut()
        .find(|c| c.id == id)
        .ok_or_else(|| EditError::UnknownChannel(id.to_owned()))
}

fn pick(c: &mut Channel, send: SendKind) -> &mut SendControl {
    match send {
        SendKind::Personal => &mut c.personal,
        SendKind::Transmission => &mut c.transmission,
    }
}

fn mic_send(p: &mut Profile, send: MicSend) -> &mut SendControl {
    match send {
        MicSend::Applications => &mut p.microphone.applications,
        MicSend::Personal => &mut p.microphone.personal,
        MicSend::Transmission => &mut p.microphone.transmission,
    }
}

fn device(key: &Option<String>) -> Result<Option<DevicePreference>, EditError> {
    match key {
        None => Ok(None),
        Some(k) if is_valid_text(k, 256) => Ok(Some(DevicePreference {
            persistent_key: k.clone(),
        })),
        Some(_) => Err(EditError::InvalidText),
    }
}

/// Aplica `cmd` sobre uma cópia e valida o resultado inteiro; o original nunca é alterado.
pub fn apply(profile: &Profile, cmd: &EditCommand) -> Result<(Profile, EditClass), EditError> {
    let mut p = profile.clone();
    let class = match cmd {
        EditCommand::SetChannelGain {
            channel: id,
            send,
            gain,
        } => {
            pick(channel(&mut p, id)?, *send).gain = *gain;
            EditClass::Continuous
        }
        EditCommand::SetChannelMute {
            channel: id,
            send,
            muted,
        } => {
            pick(channel(&mut p, id)?, *send).muted = *muted;
            EditClass::Continuous
        }
        EditCommand::SetChannelEnabled {
            channel: id,
            send,
            enabled,
        } => {
            pick(channel(&mut p, id)?, *send).enabled = *enabled;
            EditClass::Continuous
        }
        EditCommand::SetMasterGain { send, gain } => {
            match send {
                SendKind::Personal => p.master.personal.gain = *gain,
                SendKind::Transmission => p.master.transmission.gain = *gain,
            }
            EditClass::Continuous
        }
        EditCommand::SetMasterMute { send, muted } => {
            match send {
                SendKind::Personal => p.master.personal.muted = *muted,
                SendKind::Transmission => p.master.transmission.muted = *muted,
            }
            EditClass::Continuous
        }
        EditCommand::SetMicGlobalMute(m) => {
            p.microphone.global_mute = *m;
            EditClass::Continuous
        }
        EditCommand::SetMicInputGain(g) => {
            p.microphone.input_gain = *g;
            EditClass::Continuous
        }
        EditCommand::SetMicSendGain { send, gain } => {
            mic_send(&mut p, *send).gain = *gain;
            EditClass::Continuous
        }
        EditCommand::SetMicSendMute { send, muted } => {
            mic_send(&mut p, *send).muted = *muted;
            EditClass::Continuous
        }
        EditCommand::SetMicSendEnabled { send, enabled } => {
            mic_send(&mut p, *send).enabled = *enabled;
            EditClass::Continuous
        }
        EditCommand::SetChatMixPosition(x) => {
            chatmix(*x).map_err(EditError::InvalidValue)?;
            p.chatmix.position = *x;
            EditClass::Continuous
        }
        EditCommand::SetChatMixChannels(pair) => {
            p.chatmix.channels = pair.clone();
            EditClass::Structural
        }
        EditCommand::SetPreferredOutput(k) => {
            p.preferred_output = device(k)?;
            EditClass::Structural
        }
        EditCommand::SetPreferredMicrophone(k) => {
            p.preferred_microphone = device(k)?;
            EditClass::Structural
        }
        EditCommand::AddChannel { id, name } => {
            if !is_valid_id(id) {
                return Err(EditError::InvalidId(id.clone()));
            }
            if !is_valid_text(name, 128) {
                return Err(EditError::InvalidText);
            }
            if p.channels.iter().any(|c| &c.id == id) {
                return Err(EditError::DuplicateChannel(id.clone()));
            }
            // Novo canal: escuta ligada, transmissão desligada (nada entra na transmissão sem o usuário pedir, spec 8.1).
            let on = SendControl {
                enabled: true,
                muted: false,
                gain: Gain::UNITY,
            };
            p.channels.push(Channel {
                id: id.clone(),
                name: name.clone(),
                personal: on,
                transmission: SendControl {
                    enabled: false,
                    ..on
                },
            });
            EditClass::Structural
        }
        EditCommand::RenameChannel { channel: id, name } => {
            if !is_valid_text(name, 128) {
                return Err(EditError::InvalidText);
            }
            channel(&mut p, id)?.name = name.clone();
            EditClass::Structural
        }
        EditCommand::RemoveChannel {
            channel: id,
            destination,
        } => {
            channel(&mut p, id)?;
            if let Some(d) = destination {
                if d == id {
                    return Err(EditError::InvalidValue(
                        "o destino das regras não pode ser o canal removido",
                    ));
                }
                channel(&mut p, d)?;
            }
            match destination {
                Some(d) => p
                    .rules
                    .iter_mut()
                    .filter(|r| &r.channel == id)
                    .for_each(|r| r.channel = d.clone()),
                None => p.rules.retain(|r| &r.channel != id),
            }
            p.channels.retain(|c| &c.id != id);
            if p.chatmix
                .channels
                .as_ref()
                .is_some_and(|(a, b)| a == id || b == id)
            {
                p.chatmix.channels = None;
            }
            EditClass::Structural
        }
        EditCommand::AssignApp {
            matcher,
            channel: target,
        } => {
            if !matcher.is_valid() {
                return Err(EditError::InvalidText);
            }
            match target {
                Some(id) => {
                    channel(&mut p, id)?;
                    match p.rules.iter_mut().find(|r| &r.matcher == matcher) {
                        Some(r) => r.channel = id.clone(),
                        None => p.rules.push(Rule {
                            matcher: matcher.clone(),
                            channel: id.clone(),
                        }),
                    }
                }
                None => p.rules.retain(|r| &r.matcher != matcher),
            }
            EditClass::Structural
        }
    };
    plan(&p).map_err(|e| EditError::Invalid(format!("{e:?}")))?;
    Ok((p, class))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::initial_profile;

    fn ch(p: &Profile, id: &str) -> Channel {
        p.channels.iter().find(|c| c.id == id).unwrap().clone()
    }

    #[test]
    fn levels_mutes_and_enables_are_continuous_and_leave_the_original_untouched() {
        let base = initial_profile();
        let g = Gain::from_db(-6.0).unwrap();
        for cmd in [
            EditCommand::SetChannelGain {
                channel: "game".into(),
                send: SendKind::Personal,
                gain: g,
            },
            EditCommand::SetChannelMute {
                channel: "game".into(),
                send: SendKind::Transmission,
                muted: true,
            },
            EditCommand::SetChannelEnabled {
                channel: "aux".into(),
                send: SendKind::Transmission,
                enabled: true,
            },
            EditCommand::SetMasterGain {
                send: SendKind::Personal,
                gain: g,
            },
            EditCommand::SetMasterMute {
                send: SendKind::Transmission,
                muted: true,
            },
            EditCommand::SetMicGlobalMute(true),
            EditCommand::SetMicInputGain(g),
            EditCommand::SetMicSendGain {
                send: MicSend::Applications,
                gain: g,
            },
            EditCommand::SetMicSendMute {
                send: MicSend::Transmission,
                muted: true,
            },
            EditCommand::SetMicSendEnabled {
                send: MicSend::Personal,
                enabled: true,
            },
            EditCommand::SetChatMixPosition(0.5),
        ] {
            let (next, class) = apply(&base, &cmd).unwrap();
            assert_eq!(class, EditClass::Continuous, "{cmd:?}");
            assert_ne!(next, base, "{cmd:?}");
        }
        assert_eq!(base, initial_profile());
        let (p, _) = apply(
            &base,
            &EditCommand::SetChannelGain {
                channel: "game".into(),
                send: SendKind::Personal,
                gain: g,
            },
        )
        .unwrap();
        assert_eq!(ch(&p, "game").personal.gain.db(), Some(-6.0));
        assert_eq!(
            ch(&p, "game").transmission.gain.db(),
            Some(0.0),
            "só o envio pedido muda"
        );
    }

    #[test]
    fn invalid_commands_fail_without_effect() {
        let base = initial_profile();
        let bad = [
            (
                EditCommand::SetChannelMute {
                    channel: "nao-existe".into(),
                    send: SendKind::Personal,
                    muted: true,
                },
                EditError::UnknownChannel("nao-existe".into()),
            ),
            (
                EditCommand::SetChatMixPosition(1.5),
                EditError::InvalidValue("ChatMix deve estar entre -1 e 1"),
            ),
            (
                EditCommand::SetChatMixPosition(f64::NAN),
                EditError::InvalidValue("ChatMix deve estar entre -1 e 1"),
            ),
            (
                EditCommand::AddChannel {
                    id: "../x".into(),
                    name: "X".into(),
                },
                EditError::InvalidId("../x".into()),
            ),
            (
                EditCommand::AddChannel {
                    id: "game".into(),
                    name: "Outro".into(),
                },
                EditError::DuplicateChannel("game".into()),
            ),
            (
                EditCommand::AddChannel {
                    id: "novo".into(),
                    name: "".into(),
                },
                EditError::InvalidText,
            ),
            (
                EditCommand::RenameChannel {
                    channel: "game".into(),
                    name: "a\nb".into(),
                },
                EditError::InvalidText,
            ),
            (
                EditCommand::SetPreferredOutput(Some("a\u{7}b".into())),
                EditError::InvalidText,
            ),
        ];
        for (cmd, err) in bad {
            assert_eq!(apply(&base, &cmd).unwrap_err(), err, "{cmd:?}");
        }
        // referência quebrada no par do ChatMix é recusada pela validação do plano
        assert!(matches!(
            apply(
                &base,
                &EditCommand::SetChatMixChannels(Some(("game".into(), "fantasma".into())))
            ),
            Err(EditError::Invalid(_))
        ));
    }

    #[test]
    fn channel_lifecycle_is_structural_and_new_channels_never_start_in_the_broadcast() {
        let base = initial_profile();
        let (p, class) = apply(
            &base,
            &EditCommand::AddChannel {
                id: "musica".into(),
                name: "Música".into(),
            },
        )
        .unwrap();
        assert_eq!(class, EditClass::Structural);
        let c = ch(&p, "musica");
        assert!(c.personal.enabled && !c.transmission.enabled);
        assert_eq!(p.channels.last().unwrap().id, "musica");

        let (p, class) = apply(
            &p,
            &EditCommand::RenameChannel {
                channel: "musica".into(),
                name: "Sons".into(),
            },
        )
        .unwrap();
        assert_eq!(
            (
                class,
                ch(&p, "musica").id.as_str(),
                ch(&p, "musica").name.as_str()
            ),
            (EditClass::Structural, "musica", "Sons")
        );

        // remover um canal do par do ChatMix desliga o ChatMix em vez de deixar referência quebrada
        let (p, class) = apply(
            &p,
            &EditCommand::RemoveChannel {
                channel: "chat".into(),
                destination: None,
            },
        )
        .unwrap();
        assert_eq!(class, EditClass::Structural);
        assert!(p.channels.iter().all(|c| c.id != "chat") && p.chatmix.channels.is_none());
    }

    #[test]
    fn devices_and_chatmix_pair_are_structural_and_can_be_cleared() {
        let base = initial_profile();
        let (p, class) = apply(
            &base,
            &EditCommand::SetPreferredOutput(Some("alsa_output.fone".into())),
        )
        .unwrap();
        assert_eq!(
            (
                class,
                p.preferred_output.as_ref().unwrap().persistent_key.as_str()
            ),
            (EditClass::Structural, "alsa_output.fone")
        );
        let (p, _) = apply(&p, &EditCommand::SetPreferredOutput(None)).unwrap();
        assert!(p.preferred_output.is_none());
        let (p, class) = apply(
            &base,
            &EditCommand::SetChatMixChannels(Some(("media".into(), "aux".into()))),
        )
        .unwrap();
        assert_eq!(
            (class, p.chatmix.channels),
            (EditClass::Structural, Some(("media".into(), "aux".into())))
        );
    }

    #[test]
    fn silence_is_accepted_as_negative_infinity_only() {
        assert_eq!(
            Gain::from_db_or_silence(f64::NEG_INFINITY).unwrap(),
            Gain::SILENCE
        );
        assert_eq!(Gain::from_db_or_silence(-12.0).unwrap().db(), Some(-12.0));
        assert!(Gain::from_db_or_silence(f64::INFINITY).is_err());
        assert!(Gain::from_db_or_silence(f64::NAN).is_err());
        assert!(Gain::from_db_or_silence(-200.0).is_err());
    }

    fn bin(b: &str) -> AppMatcher {
        AppMatcher {
            binary: Some(b.into()),
            ..Default::default()
        }
    }

    #[test]
    fn assigning_an_app_creates_replaces_and_removes_one_rule_per_matcher() {
        let base = initial_profile();
        let (p, class) = apply(
            &base,
            &EditCommand::AssignApp {
                matcher: bin("zen"),
                channel: Some("media".into()),
            },
        )
        .unwrap();
        assert_eq!((class, p.rules.len()), (EditClass::Structural, 1));
        // mover de novo troca a regra, não duplica
        let (p, _) = apply(
            &p,
            &EditCommand::AssignApp {
                matcher: bin("zen"),
                channel: Some("game".into()),
            },
        )
        .unwrap();
        assert_eq!((p.rules.len(), p.rules[0].channel.as_str()), (1, "game"));
        let (p, _) = apply(
            &p,
            &EditCommand::AssignApp {
                matcher: bin("cider"),
                channel: Some("media".into()),
            },
        )
        .unwrap();
        assert_eq!(p.rules.len(), 2);
        // None devolve o aplicativo a Não atribuídos (apaga a regra)
        let (p, _) = apply(
            &p,
            &EditCommand::AssignApp {
                matcher: bin("zen"),
                channel: None,
            },
        )
        .unwrap();
        assert_eq!(p.rules.len(), 1);
        // erros: canal inexistente e matcher vazio ou com caractere de controle
        assert_eq!(
            apply(
                &base,
                &EditCommand::AssignApp {
                    matcher: bin("zen"),
                    channel: Some("fantasma".into())
                }
            )
            .unwrap_err(),
            EditError::UnknownChannel("fantasma".into())
        );
        assert_eq!(
            apply(
                &base,
                &EditCommand::AssignApp {
                    matcher: AppMatcher::default(),
                    channel: Some("game".into())
                }
            )
            .unwrap_err(),
            EditError::InvalidText
        );
        assert_eq!(
            apply(
                &base,
                &EditCommand::AssignApp {
                    matcher: bin("a\nb"),
                    channel: None
                }
            )
            .unwrap_err(),
            EditError::InvalidText
        );
    }

    #[test]
    fn removing_a_channel_moves_or_drops_its_rules_only_as_told() {
        let base = initial_profile();
        let (p, _) = apply(
            &base,
            &EditCommand::AssignApp {
                matcher: bin("zen"),
                channel: Some("aux".into()),
            },
        )
        .unwrap();
        let (p, _) = apply(
            &p,
            &EditCommand::AssignApp {
                matcher: bin("cider"),
                channel: Some("media".into()),
            },
        )
        .unwrap();
        // destino explícito: as regras do canal removido vão para ele; as outras ficam
        let (moved, _) = apply(
            &p,
            &EditCommand::RemoveChannel {
                channel: "aux".into(),
                destination: Some("game".into()),
            },
        )
        .unwrap();
        let by = |p: &Profile, b: &str| {
            p.rules
                .iter()
                .find(|r| r.matcher.binary.as_deref() == Some(b))
                .map(|r| r.channel.clone())
        };
        assert_eq!(
            (by(&moved, "zen").as_deref(), by(&moved, "cider").as_deref()),
            (Some("game"), Some("media"))
        );
        // sem destino: as regras do canal removido somem (Não atribuídos); as outras ficam
        let (dropped, _) = apply(
            &p,
            &EditCommand::RemoveChannel {
                channel: "aux".into(),
                destination: None,
            },
        )
        .unwrap();
        assert_eq!(
            (by(&dropped, "zen"), by(&dropped, "cider").as_deref()),
            (None, Some("media"))
        );
        // destino inválido: o próprio canal ou um que não existe
        assert!(matches!(
            apply(
                &p,
                &EditCommand::RemoveChannel {
                    channel: "aux".into(),
                    destination: Some("aux".into())
                }
            ),
            Err(EditError::InvalidValue(_))
        ));
        assert_eq!(
            apply(
                &p,
                &EditCommand::RemoveChannel {
                    channel: "aux".into(),
                    destination: Some("fantasma".into())
                }
            )
            .unwrap_err(),
            EditError::UnknownChannel("fantasma".into())
        );
    }
}
