//! Medidores de nível (spec 5, 9): cálculo puro dos níveis por slider a partir dos picos medidos nos barramentos do grafo.
//!
//! Medição: um tap por barramento (canais, MASTER, MIC comum, MIC para aplicativos) mede o **pico de amostra** num intervalo.
//! O nível de cada slider é o pico do barramento que o alimenta vezes a amplitude efetiva do envio (ganho, mute, habilitação e
//! ChatMix, tudo lido do plano, a mesma fonte do áudio): como o ganho é linear, o resultado é exatamente o pico depois do envio.
//! O medidor independe da posição do slider. Piso visual −60 dBFS; pico mantido 1 s; indicador de clipe (≥ 0 dBFS) mantido 2 s.

use crate::topology::{
    channel_node, plan, BranchSpec, Plan, MASTER_PERSONAL, MASTER_TRANSMISSION, MIC_APPS,
    MIC_COMMON, MIX_PERSONAL, MIX_TRANSMISSION,
};
use crate::Profile;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Piso visual do medidor em dBFS.
pub const FLOOR_DB: f32 = -60.0;
pub const PEAK_HOLD: Duration = Duration::from_secs(1);
pub const CLIP_HOLD: Duration = Duration::from_secs(2);
/// Queda do nível mostrado, em dB por segundo (ataque instantâneo).
pub const FALL_DB_PER_S: f32 = 40.0;

/// Qual slider um nível alimenta.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MeterKey {
    /// Envio pessoal (`transmission = false`) ou de transmissão de um canal.
    Channel { id: String, transmission: bool },
    /// Saída do MASTER (depois do ganho do MASTER).
    Master { transmission: bool },
    /// Envios do MIC (depois do ganho comum e do mute global, vezes o ganho do envio).
    Mic { transmission: bool },
    /// Ramo "Microfone para aplicativos".
    MicApps,
}

/// Nomes dos barramentos a medir para o perfil (um tap por nome).
pub fn tap_buses(profile: &Profile) -> Vec<String> {
    let mut v: Vec<String> = profile
        .channels
        .iter()
        .map(|c| channel_node(&c.id))
        .collect();
    v.extend([MASTER_PERSONAL, MASTER_TRANSMISSION, MIC_COMMON, MIC_APPS].map(str::to_owned));
    v
}

/// Pico linear → dBFS; `-inf` para zero.
pub fn peak_to_db(peak: f32) -> f32 {
    if peak <= 0.0 {
        f32::NEG_INFINITY
    } else {
        20.0 * peak.log10()
    }
}

/// dBFS → posição 0..=1 na barra (piso −60 dBFS = 0; 0 dBFS = 1).
pub fn db_to_position(db: f32) -> f32 {
    ((db - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0)
}

fn send_amplitude(plan: &Plan, from: &str, to: &str) -> f32 {
    plan.branches
        .iter()
        .find(|b: &&BranchSpec| b.from == from && b.to == to)
        .map_or(0.0, |b| if b.muted { 0.0 } else { b.volume as f32 })
}

/// Pico (linear) por slider, dados os picos medidos por barramento (`node.name` → pico). Barramento sem medida conta como 0.
/// Envio desabilitado ou mutado dá 0 (silêncio visível), porque o áudio realmente não passa por ele.
pub fn strip_peaks(profile: &Profile, bus_peaks: &HashMap<String, f32>) -> HashMap<MeterKey, f32> {
    let peak = |name: &str| bus_peaks.get(name).copied().unwrap_or(0.0);
    let mut out = HashMap::new();
    let Ok(plan) = plan(profile) else { return out };
    for c in &profile.channels {
        let bus = channel_node(&c.id);
        let p = peak(&bus);
        out.insert(
            MeterKey::Channel {
                id: c.id.clone(),
                transmission: false,
            },
            p * send_amplitude(&plan, &bus, MIX_PERSONAL),
        );
        out.insert(
            MeterKey::Channel {
                id: c.id.clone(),
                transmission: true,
            },
            p * send_amplitude(&plan, &bus, MIX_TRANSMISSION),
        );
    }
    out.insert(
        MeterKey::Master {
            transmission: false,
        },
        peak(MASTER_PERSONAL),
    );
    out.insert(
        MeterKey::Master { transmission: true },
        peak(MASTER_TRANSMISSION),
    );
    let mic = peak(MIC_COMMON);
    out.insert(
        MeterKey::Mic {
            transmission: false,
        },
        mic * send_amplitude(&plan, MIC_COMMON, MIX_PERSONAL),
    );
    out.insert(
        MeterKey::Mic { transmission: true },
        mic * send_amplitude(&plan, MIC_COMMON, MIX_TRANSMISSION),
    );
    out.insert(MeterKey::MicApps, peak(MIC_APPS));
    out
}

/// Estado de exibição de um medidor: nível com queda suave, pico mantido e clipe mantido.
#[derive(Clone, Copy, Debug)]
pub struct Meter {
    /// Nível mostrado (dBFS), no piso quando silencioso.
    pub level_db: f32,
    /// Pico mantido (dBFS) e até quando.
    pub hold_db: f32,
    hold_until: Option<Instant>,
    clip_until: Option<Instant>,
    last: Option<Instant>,
}

impl Default for Meter {
    fn default() -> Self {
        Self {
            level_db: FLOOR_DB,
            hold_db: FLOOR_DB,
            hold_until: None,
            clip_until: None,
            last: None,
        }
    }
}

impl Meter {
    /// Atualiza com um novo pico linear (ou `None` se nada chegou, só deixa cair). Ataque instantâneo; queda de
    /// `FALL_DB_PER_S`; pico mantido por 1 s; clipe (≥ 0 dBFS) aceso por 2 s.
    pub fn update(&mut self, peak: Option<f32>, now: Instant) {
        let dt = self
            .last
            .map_or(0.0, |t| now.saturating_duration_since(t).as_secs_f32());
        self.last = Some(now);
        let fallen = (self.level_db - FALL_DB_PER_S * dt).max(FLOOR_DB);
        let new = peak.map_or(f32::NEG_INFINITY, peak_to_db).max(FLOOR_DB);
        self.level_db = new.max(fallen);
        if self.level_db >= self.hold_db || self.hold_until.is_none_or(|t| now >= t) {
            if self.level_db >= self.hold_db || self.hold_until.is_some() {
                self.hold_db = self
                    .level_db
                    .max(if self.hold_until.is_some_and(|t| now < t) {
                        self.hold_db
                    } else {
                        FLOOR_DB
                    });
            }
            if new >= self.hold_db - f32::EPSILON || self.hold_until.is_none_or(|t| now >= t) {
                self.hold_until = Some(now + PEAK_HOLD);
            }
        }
        if peak.is_some_and(|p| peak_to_db(p) >= 0.0) {
            self.clip_until = Some(now + CLIP_HOLD);
        }
        if self.hold_until.is_some_and(|t| now >= t) && self.level_db < self.hold_db {
            self.hold_db = self.level_db;
            self.hold_until = None;
        }
    }

    pub fn clipping(&self, now: Instant) -> bool {
        self.clip_until.is_some_and(|t| now < t)
    }

    /// Tudo no piso e sem clipe: a interface pode parar de animar.
    pub fn is_idle(&self, now: Instant) -> bool {
        self.level_db <= FLOOR_DB && self.hold_db <= FLOOR_DB && !self.clipping(now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::{apply, EditCommand, SendKind};
    use crate::{initial_profile, Gain};

    fn peaks(pairs: &[(&str, f32)]) -> HashMap<String, f32> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect()
    }

    fn key(id: &str, tx: bool) -> MeterKey {
        MeterKey::Channel {
            id: id.into(),
            transmission: tx,
        }
    }

    #[test]
    fn db_mapping_uses_the_sixty_db_floor() {
        assert_eq!(peak_to_db(1.0), 0.0);
        assert!((peak_to_db(0.5) + 6.0206).abs() < 1e-3);
        assert_eq!(peak_to_db(0.0), f32::NEG_INFINITY);
        assert_eq!(db_to_position(0.0), 1.0);
        assert_eq!(db_to_position(-60.0), 0.0);
        assert!((db_to_position(-30.0) - 0.5).abs() < 1e-6);
        assert_eq!(db_to_position(f32::NEG_INFINITY), 0.0);
        assert_eq!(db_to_position(6.0), 1.0, "acima de 0 dBFS satura");
    }

    #[test]
    fn a_slider_meter_is_the_bus_peak_times_the_send_and_ignores_the_slider_position_of_other_sends(
    ) {
        let mut p = initial_profile();
        let (p1, _) = apply(
            &p,
            &EditCommand::SetChannelGain {
                channel: "game".into(),
                send: SendKind::Personal,
                gain: Gain::from_db(-6.0).unwrap(),
            },
        )
        .unwrap();
        p = p1;
        let bus = peaks(&[("iara.ch.game", 0.5)]);
        let m = strip_peaks(&p, &bus);
        // escuta com −6 dB: metade; transmissão a 0 dB: igual ao barramento (cada envio tem o seu nível)
        assert!((m[&key("game", false)] - 0.5 * 0.501_187).abs() < 1e-4);
        assert!((m[&key("game", true)] - 0.5).abs() < 1e-6);
        // canal sem sinal: zero
        assert_eq!(m[&key("chat", false)], 0.0);
    }

    #[test]
    fn mute_disable_and_chatmix_are_reflected_because_they_come_from_the_plan() {
        let p = initial_profile();
        let bus = peaks(&[("iara.ch.game", 1.0), ("iara.ch.aux", 1.0)]);
        let (muted, _) = apply(
            &p,
            &EditCommand::SetChannelMute {
                channel: "game".into(),
                send: SendKind::Personal,
                muted: true,
            },
        )
        .unwrap();
        assert_eq!(
            strip_peaks(&muted, &bus)[&key("game", false)],
            0.0,
            "mutado: o áudio não passa, o medidor cai a zero"
        );
        assert_eq!(
            strip_peaks(&muted, &bus)[&key("game", true)],
            1.0,
            "o outro envio segue"
        );
        assert_eq!(
            strip_peaks(&p, &bus)[&key("aux", true)],
            0.0,
            "AUX nasce fora da transmissão"
        );
        // ChatMix atenua só a escuta do canal favorecido do outro lado
        let (cm, _) = apply(&p, &EditCommand::SetChatMixPosition(1.0)).unwrap();
        assert_eq!(
            strip_peaks(&cm, &bus)[&key("game", false)],
            0.0,
            "ChatMix +1 zera o GAME na escuta"
        );
        assert_eq!(
            strip_peaks(&cm, &bus)[&key("game", true)],
            1.0,
            "transmissão não recebe ChatMix"
        );
    }

    #[test]
    fn master_and_mic_strips_read_their_own_buses() {
        let p = initial_profile();
        let bus = peaks(&[
            ("iara.master.personal", 0.25),
            ("iara.master.transmission", 0.125),
            ("iara.mic.common", 0.5),
            ("iara.mic.apps", 0.4),
        ]);
        let m = strip_peaks(&p, &bus);
        assert_eq!(
            m[&MeterKey::Master {
                transmission: false
            }],
            0.25
        );
        assert_eq!(m[&MeterKey::Master { transmission: true }], 0.125);
        assert_eq!(m[&MeterKey::MicApps], 0.4);
        // MIC: escuta desabilitada por padrão (0), transmissão a 0 dB (igual ao barramento comum)
        assert_eq!(
            m[&MeterKey::Mic {
                transmission: false
            }],
            0.0
        );
        assert_eq!(m[&MeterKey::Mic { transmission: true }], 0.5);
        // mute global do MIC zera o barramento comum antes da divisão: o motor já mede zero; aqui um envio mutado também zera
        let (mm, _) = apply(
            &p,
            &EditCommand::SetMicSendMute {
                send: crate::edit::MicSend::Transmission,
                muted: true,
            },
        )
        .unwrap();
        assert_eq!(
            strip_peaks(&mm, &bus)[&MeterKey::Mic { transmission: true }],
            0.0
        );
    }

    #[test]
    fn the_taps_cover_every_bus_that_feeds_a_slider() {
        let p = initial_profile();
        let taps = tap_buses(&p);
        for need in [
            "iara.ch.game",
            "iara.ch.chat",
            "iara.ch.media",
            "iara.ch.aux",
            "iara.master.personal",
            "iara.master.transmission",
            "iara.mic.common",
            "iara.mic.apps",
        ] {
            assert!(taps.iter().any(|t| t == need), "{need}");
        }
        assert_eq!(taps.len(), 8);
    }

    fn t(base: Instant, ms: u64) -> Instant {
        base + Duration::from_millis(ms)
    }

    #[test]
    fn the_displayed_level_attacks_instantly_falls_smoothly_and_holds_the_peak_for_a_second() {
        let t0 = Instant::now();
        let mut m = Meter::default();
        m.update(Some(0.5), t0); // −6 dBFS
        assert!((m.level_db + 6.02).abs() < 0.05 && (m.hold_db + 6.02).abs() < 0.05);
        // silêncio: o nível cai 40 dB/s, o pico fica
        m.update(None, t(t0, 250));
        assert!((m.level_db - (-6.02 - 10.0)).abs() < 0.1, "{}", m.level_db);
        assert!((m.hold_db + 6.02).abs() < 0.05, "pico mantido");
        m.update(None, t(t0, 900));
        assert!((m.hold_db + 6.02).abs() < 0.05, "ainda dentro de 1 s");
        m.update(None, t(t0, 1_100));
        assert!(m.hold_db < -6.5, "passado 1 s o pico solta: {}", m.hold_db);
        // longe o bastante: tudo no piso e ocioso
        m.update(None, t(t0, 4_000));
        assert!(m.is_idle(t(t0, 4_000)));
    }

    #[test]
    fn clipping_lights_for_two_seconds_and_a_new_peak_restarts_the_hold() {
        let t0 = Instant::now();
        let mut m = Meter::default();
        m.update(Some(1.0), t0); // 0 dBFS
        assert!(m.clipping(t0));
        m.update(None, t(t0, 1_900));
        assert!(m.clipping(t(t0, 1_900)));
        m.update(None, t(t0, 2_100));
        assert!(!m.clipping(t(t0, 2_100)));
        // um pico novo e mais alto reinicia a espera
        let mut n = Meter::default();
        n.update(Some(0.25), t0);
        n.update(Some(0.5), t(t0, 900));
        n.update(None, t(t0, 1_800));
        assert!(
            (n.hold_db + 6.02).abs() < 0.1,
            "o pico novo vale mais 1 s: {}",
            n.hold_db
        );
        // sinal abaixo do piso nunca acende nada
        let mut q = Meter::default();
        q.update(Some(0.0001), t0);
        assert!(q.is_idle(t0));
    }
}
