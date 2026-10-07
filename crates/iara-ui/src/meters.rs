//! Estado de exibição dos medidores da janela: recebe os picos por slider do serviço e entrega, para cada slider, a posição do
//! nível, a do pico mantido e o indicador de clipe. Puro (sem GTK): a decaída e a espera vêm de `iara_core::meter::Meter`.

use iara_core::meter::{db_to_position, Meter, MeterKey};
use std::collections::HashMap;
use std::time::Instant;

/// O que a barra de um slider desenha (posições de 0 a 1 no eixo do slider: 0 = −60 dBFS, 1 = 0 dBFS).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MeterView {
    pub level: f32,
    pub hold: f32,
    pub clip: bool,
}

#[derive(Default)]
pub struct MeterBoard {
    meters: HashMap<MeterKey, Meter>,
}

impl MeterBoard {
    /// Aplica um pacote do serviço (chave em texto, pico linear). Chaves desconhecidas são ignoradas; sliders ausentes do
    /// pacote só decaem.
    pub fn feed(&mut self, levels: &[(String, f64)], now: Instant) {
        let mut seen = Vec::with_capacity(levels.len());
        for (k, v) in levels {
            let Some(key) = MeterKey::parse(k) else {
                continue;
            };
            // só valores finitos e não negativos entram (um pacote corrompido não pode travar a barra no topo)
            let peak = if v.is_finite() && *v >= 0.0 {
                *v as f32
            } else {
                0.0
            };
            self.meters
                .entry(key.clone())
                .or_default()
                .update(Some(peak), now);
            seen.push(key);
        }
        for (k, m) in &mut self.meters {
            if !seen.contains(k) {
                m.update(None, now);
            }
        }
    }

    /// Passa o tempo sem notícias: tudo decai.
    pub fn tick(&mut self, now: Instant) {
        for m in self.meters.values_mut() {
            m.update(None, now);
        }
    }

    /// Esquece tudo (serviço sumiu, medidores desligados): as barras voltam ao piso na hora.
    pub fn clear(&mut self) {
        self.meters.clear();
    }

    pub fn view(&self, key: &MeterKey, now: Instant) -> MeterView {
        self.meters
            .get(key)
            .map_or_else(MeterView::default, |m| MeterView {
                level: db_to_position(m.level_db),
                hold: db_to_position(m.hold_db),
                clip: m.clipping(now),
            })
    }

    /// Nada para animar: tudo no piso e sem clipe aceso.
    pub fn is_idle(&self, now: Instant) -> bool {
        self.meters.values().all(|m| m.is_idle(now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn key(id: &str) -> MeterKey {
        MeterKey::Channel {
            id: id.into(),
            transmission: false,
        }
    }

    fn pkt(pairs: &[(&str, f64)]) -> Vec<(String, f64)> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect()
    }

    #[test]
    fn a_packet_moves_only_the_sliders_it_names_and_the_rest_decay() {
        let t0 = Instant::now();
        let mut b = MeterBoard::default();
        b.feed(
            &pkt(&[("ch:game:personal", 0.5), ("ch:chat:personal", 0.1)]),
            t0,
        );
        let v = b.view(&key("game"), t0);
        assert!((v.level - db_to_position(-6.02)).abs() < 0.01, "{v:?}");
        // próximo pacote só cita o game: o chat decai 40 dB/s (0,25 s = 10 dB)
        let t1 = t0 + Duration::from_millis(250);
        b.feed(&pkt(&[("ch:game:personal", 0.5)]), t1);
        let chat = b.view(&key("chat"), t1);
        assert!(
            (chat.level - db_to_position(-20.0 - 10.0)).abs() < 0.02,
            "{chat:?}"
        );
    }

    #[test]
    fn unknown_or_corrupt_entries_are_ignored_or_harmless() {
        let t0 = Instant::now();
        let mut b = MeterBoard::default();
        b.feed(
            &pkt(&[
                ("lixo", 1.0),
                ("ch:game:personal", f64::NAN),
                ("ch:chat:personal", -3.0),
                ("ch:aux:personal", f64::INFINITY),
            ]),
            t0,
        );
        assert!(b.is_idle(t0), "nada válido chegou");
        assert_eq!(b.view(&key("game"), t0), MeterView::default());
    }

    #[test]
    fn clipping_shows_and_the_board_goes_idle_after_everything_decays() {
        let t0 = Instant::now();
        let mut b = MeterBoard::default();
        b.feed(&pkt(&[("master:personal", 1.0)]), t0);
        let m = MeterKey::Master {
            transmission: false,
        };
        assert!(b.view(&m, t0).clip);
        assert!(!b.is_idle(t0));
        b.tick(t0 + Duration::from_secs(1));
        assert!(
            b.view(&m, t0 + Duration::from_secs(1)).clip,
            "clipe dura 2 s"
        );
        b.tick(t0 + Duration::from_secs(3));
        b.tick(t0 + Duration::from_secs(6));
        assert!(b.is_idle(t0 + Duration::from_secs(6)));
        assert_eq!(
            b.view(&m, t0 + Duration::from_secs(6)),
            MeterView::default()
        );
    }

    #[test]
    fn clear_drops_everything_at_once() {
        let t0 = Instant::now();
        let mut b = MeterBoard::default();
        b.feed(&pkt(&[("mic-apps", 0.9)]), t0);
        b.clear();
        assert!(b.is_idle(t0));
    }
}
