//! Formato em disco (schema 1), separado do domínio: o arquivo pode evoluir sem arrastar `iara-core`.
//! Ganho: `gain_db` (finito, −60..0) ou `silence = true`; nunca infinito em TOML (spec 5).

use iara_core::{
    topology::plan, Channel, ChatMixSetting, DevicePreference, Gain, Master, Microphone, Profile,
    SendControl,
};
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SendDto {
    pub enabled: bool,
    pub muted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gain_db: Option<f64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub silence: bool,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ChannelDto {
    pub id: String,
    pub name: String,
    pub personal: SendDto,
    pub transmission: SendDto,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MasterDto {
    pub personal: SendDto,
    pub transmission: SendDto,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MicrophoneDto {
    pub global_mute: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_gain_db: Option<f64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub input_silence: bool,
    pub personal: SendDto,
    pub transmission: SendDto,
    pub applications: SendDto,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ChatMixDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channels: Option<[String; 2]>,
    pub position: f64,
}

#[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DevicesDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub microphone: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProfileDto {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub master: MasterDto,
    pub microphone: MicrophoneDto,
    pub chatmix: ChatMixDto,
    #[serde(default)]
    pub devices: DevicesDto,
    #[serde(default, rename = "channels")]
    pub channels: Vec<ChannelDto>,
}

fn send_to_dto(s: &SendControl) -> SendDto {
    SendDto {
        enabled: s.enabled,
        muted: s.muted,
        gain_db: s.gain.db(),
        silence: s.gain.db().is_none(),
    }
}

fn gain_from(db: Option<f64>, silence: bool, what: &str) -> Result<Gain, String> {
    match (db, silence) {
        (_, true) => Ok(Gain::SILENCE),
        (Some(db), false) => Gain::from_db(db).map_err(|e| format!("{what}: {e}")),
        (None, false) => Err(format!("{what}: falta gain_db ou silence = true")),
    }
}

fn send_from_dto(d: &SendDto, what: &str) -> Result<SendControl, String> {
    Ok(SendControl {
        enabled: d.enabled,
        muted: d.muted,
        gain: gain_from(d.gain_db, d.silence, what)?,
    })
}

pub fn profile_to_dto(p: &Profile) -> ProfileDto {
    let m = &p.microphone;
    ProfileDto {
        schema_version: SCHEMA_VERSION,
        id: p.id.clone(),
        name: p.name.clone(),
        master: MasterDto {
            personal: send_to_dto(&p.master.personal),
            transmission: send_to_dto(&p.master.transmission),
        },
        microphone: MicrophoneDto {
            global_mute: m.global_mute,
            input_gain_db: m.input_gain.db(),
            input_silence: m.input_gain.db().is_none(),
            personal: send_to_dto(&m.personal),
            transmission: send_to_dto(&m.transmission),
            applications: send_to_dto(&m.applications),
        },
        chatmix: ChatMixDto {
            channels: p.chatmix.channels.clone().map(|(a, b)| [a, b]),
            position: p.chatmix.position,
        },
        devices: DevicesDto {
            output: p
                .preferred_output
                .as_ref()
                .map(|d| d.persistent_key.clone()),
            microphone: p
                .preferred_microphone
                .as_ref()
                .map(|d| d.persistent_key.clone()),
        },
        channels: p
            .channels
            .iter()
            .map(|c| ChannelDto {
                id: c.id.clone(),
                name: c.name.clone(),
                personal: send_to_dto(&c.personal),
                transmission: send_to_dto(&c.transmission),
            })
            .collect(),
    }
}

/// Converte e valida: ganhos finitos na faixa, ids seguros, referências do ChatMix e unicidade de canais (via `plan`).
pub fn profile_from_dto(d: ProfileDto) -> Result<Profile, String> {
    if !iara_core::is_valid_text(&d.name, 128) {
        return Err("nome do perfil inválido".into());
    }
    let mut channels = Vec::with_capacity(d.channels.len());
    for c in d.channels {
        if !iara_core::is_valid_text(&c.name, 128) {
            return Err(format!("canal {}: nome inválido", c.id));
        }
        channels.push(Channel {
            personal: send_from_dto(&c.personal, &format!("canal {} (escuta)", c.id))?,
            transmission: send_from_dto(&c.transmission, &format!("canal {} (transmissão)", c.id))?,
            id: c.id,
            name: c.name,
        });
    }
    let m = d.microphone;
    let profile = Profile {
        id: d.id,
        name: d.name,
        channels,
        microphone: Microphone {
            input_gain: gain_from(m.input_gain_db, m.input_silence, "microfone (entrada)")?,
            global_mute: m.global_mute,
            personal: send_from_dto(&m.personal, "microfone (escuta)")?,
            transmission: send_from_dto(&m.transmission, "microfone (transmissão)")?,
            applications: send_from_dto(&m.applications, "microfone (aplicativos)")?,
        },
        master: Master {
            personal: send_from_dto(&d.master.personal, "master (escuta)")?,
            transmission: send_from_dto(&d.master.transmission, "master (transmissão)")?,
        },
        chatmix: ChatMixSetting {
            channels: d.chatmix.channels.map(|[a, b]| (a, b)),
            position: d.chatmix.position,
        },
        preferred_output: d
            .devices
            .output
            .map(|k| DevicePreference { persistent_key: k }),
        preferred_microphone: d
            .devices
            .microphone
            .map(|k| DevicePreference { persistent_key: k }),
    };
    plan(&profile).map_err(|e| format!("perfil inválido: {e:?}"))?;
    Ok(profile)
}

/// Configuração global (spec 7 e 8.12).
#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
#[serde(deny_unknown_fields)]
pub struct GlobalConfig {
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_profile: Option<String>,
    /// Iniciar o serviço com a sessão (spec 8.7).
    pub autostart: bool,
    /// Compartilhar a saída física escolhida entre todos os perfis (spec 8.12).
    pub share_output_device: bool,
    /// Compartilhar o microfone físico escolhido entre todos os perfis.
    pub share_microphone_device: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared_output_device: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared_microphone_device: Option<String>,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            active_profile: None,
            autostart: true,
            share_output_device: false,
            share_microphone_device: false,
            shared_output_device: None,
            shared_microphone_device: None,
        }
    }
}

impl GlobalConfig {
    pub fn validate(&self) -> Result<(), String> {
        if let Some(id) = &self.active_profile {
            if !iara_core::is_valid_id(id) {
                return Err("perfil ativo com id inválido".into());
            }
        }
        for d in [&self.shared_output_device, &self.shared_microphone_device]
            .into_iter()
            .flatten()
        {
            if !iara_core::is_valid_text(d, 256) {
                return Err("dispositivo compartilhado inválido".into());
            }
        }
        Ok(())
    }
}
