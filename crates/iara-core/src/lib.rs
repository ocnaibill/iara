//! Domínio compartilhado; sem dependência de GTK, PipeWire ou IPC.

pub mod topology;

pub const HISTORY_LIMIT: usize = 50;

/// Ids lógicos (perfil, canal) viram nomes de arquivo e de nó do PipeWire: só `[a-z0-9_-]`, 1 a 32 caracteres,
/// começando por letra ou dígito. Perfis importados são entrada não confiável.
pub fn is_valid_id(id: &str) -> bool {
    let mut chars = id.chars();
    id.len() <= 32
        && chars
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// Chave de dispositivo/texto livre: sem caracteres de controle e de tamanho razoável.
pub fn is_valid_text(text: &str, max_len: usize) -> bool {
    !text.is_empty() && text.len() <= max_len && !text.chars().any(char::is_control)
}
pub const SPEC_VERSION: &str = "0.10";

/// Ganho digital de amplitude; None representa silêncio (-infinito dB).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gain(Option<f64>);

impl Gain {
    pub const UNITY: Self = Self(Some(0.0));
    pub const SILENCE: Self = Self(None);

    pub fn from_db(db: f64) -> Result<Self, &'static str> {
        if !db.is_finite() || !(-60.0..=0.0).contains(&db) {
            return Err("ganho deve ser finito e estar entre -60 e 0 dB");
        }
        Ok(Self(Some(db)))
    }

    pub fn db(self) -> Option<f64> {
        self.0
    }

    pub fn amplitude(self) -> f64 {
        self.0.map_or(0.0, |db| 10.0_f64.powf(db / 20.0))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SendControl {
    pub enabled: bool,
    pub muted: bool,
    pub gain: Gain,
}

impl SendControl {
    pub fn effective_amplitude(self) -> f64 {
        if self.enabled && !self.muted {
            self.gain.amplitude()
        } else {
            0.0
        }
    }
}

/// Ambos os envios podem estar habilitados ao mesmo tempo.
#[derive(Clone, Debug, PartialEq)]
pub struct Channel {
    pub id: String,
    pub name: String,
    pub personal: SendControl,
    pub transmission: SendControl,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Microphone {
    pub input_gain: Gain,
    pub global_mute: bool,
    pub personal: SendControl,
    pub transmission: SendControl,
    pub applications: SendControl,
}

impl Microphone {
    pub fn effective_amplitudes(self) -> [f64; 3] {
        let common = if self.global_mute {
            0.0
        } else {
            self.input_gain.amplitude()
        };
        [self.personal, self.transmission, self.applications]
            .map(|send| common * send.effective_amplitude())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoutingPolicy {
    RespectExternal,
    AlwaysFollowMixer,
}

/// IDs do domínio, nunca IDs numéricos temporários do PipeWire.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DevicePreference {
    pub persistent_key: String,
}

/// MASTER age depois da soma de cada mix (ganho e mute próprios por mix).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Master {
    pub personal: SendControl,
    pub transmission: SendControl,
}

/// Par de canais (ids) equilibrado pelo ChatMix e posição atual em [-1, 1];
/// -1 favorece o primeiro canal, +1 o segundo. `channels = None` desativa o ChatMix.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatMixSetting {
    pub channels: Option<(String, String)>,
    pub position: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub channels: Vec<Channel>,
    pub microphone: Microphone,
    pub master: Master,
    pub chatmix: ChatMixSetting,
    pub preferred_output: Option<DevicePreference>,
    pub preferred_microphone: Option<DevicePreference>,
}

/// x=-1 favorece GAME; x=+1 favorece CHAT. Retorna (GAME, CHAT).
/// Multiplicadores adicionais da escuta; não alteram ganhos salvos.
pub fn chatmix(x: f64) -> Result<(f64, f64), &'static str> {
    if !x.is_finite() || !(-1.0..=1.0).contains(&x) {
        return Err("ChatMix deve estar entre -1 e 1");
    }
    let attenuation = if x.abs() == 1.0 {
        0.0
    } else {
        (std::f64::consts::FRAC_PI_2 * x.abs()).cos()
    };
    Ok(if x >= 0.0 {
        (attenuation, 1.0)
    } else {
        (1.0, attenuation)
    })
}

pub fn initial_profile() -> Profile {
    let send = |enabled| SendControl {
        enabled,
        muted: false,
        gain: Gain::UNITY,
    };
    Profile {
        id: "default".into(),
        name: "Padrão".into(),
        channels: ["GAME", "CHAT", "MEDIA", "AUX"]
            .into_iter()
            .map(|name| Channel {
                id: name.to_ascii_lowercase(),
                name: name.into(),
                personal: send(true),
                transmission: send(name != "AUX"),
            })
            .collect(),
        microphone: Microphone {
            input_gain: Gain::UNITY,
            global_mute: false,
            personal: send(false),
            transmission: send(true),
            applications: send(true),
        },
        master: Master {
            personal: send(true),
            transmission: send(true),
        },
        chatmix: ChatMixSetting {
            channels: Some(("game".into(), "chat".into())),
            position: 0.0,
        },
        preferred_output: None,
        preferred_microphone: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chatmix_is_symmetric_and_never_boosts() {
        assert_eq!(chatmix(0.0), Ok((1.0, 1.0)));
        assert_eq!(chatmix(-1.0), Ok((1.0, 0.0)));
        assert_eq!(chatmix(1.0), Ok((0.0, 1.0)));
        for step in 0..=100 {
            let x = step as f64 / 100.0;
            let (a, b) = chatmix(x).unwrap();
            assert!((0.0..=1.0).contains(&a) && b == 1.0);
            assert_eq!(chatmix(-x).unwrap(), (b, a));
        }
        assert!(chatmix(f64::NAN).is_err());
    }

    #[test]
    fn microphone_sends_and_global_mute_are_independent() {
        let mut mic = initial_profile().microphone;
        mic.transmission.muted = true;
        assert_eq!(mic.effective_amplitudes(), [0.0, 0.0, 1.0]);
        mic.personal.enabled = true;
        mic.personal.gain = Gain::from_db(-20.0).unwrap();
        assert_eq!(mic.effective_amplitudes(), [0.1, 0.0, 1.0]);
        mic.global_mute = true;
        assert_eq!(mic.effective_amplitudes(), [0.0; 3]);
    }

    #[test]
    fn ids_are_restricted_to_a_safe_alphabet() {
        for ok in ["game", "a", "chat-2", "x_y", "0abc"] {
            assert!(is_valid_id(ok), "{ok}");
        }
        for bad in [
            "",
            "-a",
            "_a",
            "A",
            "a b",
            "a/b",
            "..",
            "a\"b",
            "ç",
            &"a".repeat(33),
        ] {
            assert!(!is_valid_id(bad), "{bad}");
        }
        assert!(is_valid_text("alsa_output.pci-0000", 64));
        assert!(!is_valid_text("a\nb", 64) && !is_valid_text("", 64) && !is_valid_text("abc", 2));
    }

    #[test]
    fn gain_rejects_invalid_values_and_preserves_silence() {
        assert_eq!(Gain::UNITY.amplitude(), 1.0);
        assert_eq!(Gain::SILENCE.amplitude(), 0.0);
        assert!((Gain::from_db(-60.0).unwrap().amplitude() - 0.001).abs() < 1e-12);
        for db in [f64::NAN, f64::INFINITY, -61.0, 1.0] {
            assert!(Gain::from_db(db).is_err());
        }
    }
}
