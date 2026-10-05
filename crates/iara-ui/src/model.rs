//! Modelo de apresentação: transforma o retrato do serviço no que a janela mostra e traduz gestos em valores de domínio.
//! Sem GTK e sem D-Bus: tudo aqui é função pura sobre `iara_ipc::State`.

use iara_core::{chatmix, Gain, Profile, SendControl};
use iara_ipc::State;

/// Menor posição positiva do slider: p = 0 é silêncio exato; qualquer p > 0 é um ganho de −60 a 0 dB (spec 5).
pub const MIN_POSITION: f64 = 1e-6;

/// Posição normalizada do slider (0..=1) → ganho. 0 = silêncio; 0 < p ≤ 1: dB = −60 + 60·p.
pub fn position_to_gain(position: f64) -> Gain {
    if !position.is_finite() || position <= 0.0 {
        return Gain::SILENCE;
    }
    Gain::from_db(-60.0 + 60.0 * position.min(1.0)).unwrap_or(Gain::SILENCE)
}

/// Ganho → posição do slider. Silêncio vai a 0; −60 dB exato vai à menor posição positiva (só o campo numérico o
/// escolhe de fato: o slider em 0 é silêncio, não −60 dB).
pub fn gain_to_position(gain: Gain) -> f64 {
    match gain.db() {
        None => 0.0,
        Some(db) => ((db + 60.0) / 60.0).clamp(MIN_POSITION, 1.0),
    }
}

/// "−6,0 dB" com o sinal de menos tipográfico; silêncio como "−∞ dB". Exibe dB, nunca percentagem (spec 5).
pub fn format_db(gain: Gain) -> String {
    match gain.db() {
        None => "−∞ dB".to_owned(),
        Some(db) => {
            let text = if db == 0.0 {
                "0.0".to_owned()
            } else {
                format!("{db:.1}")
            };
            format!("{} dB", text.replace('-', "−").replace('.', ","))
        }
    }
}

/// Atenuação adicional do ChatMix, a mostrar ao lado do valor salvo (nunca somada visualmente): `None` se não atenua.
pub fn format_chatmix_attenuation(factor: f64) -> Option<String> {
    if factor >= 1.0 {
        None
    } else if factor <= 0.0 {
        Some("ChatMix −∞ dB".to_owned())
    } else {
        Some(
            format!("ChatMix {:.1} dB", 20.0 * factor.log10())
                .replace('-', "−")
                .replace('.', ","),
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SendView {
    pub enabled: bool,
    pub muted: bool,
    pub position: f64,
    pub label: String,
    /// Atenuação adicional do ChatMix (só no envio pessoal dos dois canais do par).
    pub note: Option<String>,
}

fn send_view(s: &SendControl, chatmix_factor: Option<f64>) -> SendView {
    SendView {
        enabled: s.enabled,
        muted: s.muted,
        position: gain_to_position(s.gain),
        label: format_db(s.gain),
        note: chatmix_factor.and_then(format_chatmix_attenuation),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ColumnKind {
    Master,
    Channel,
    Mic,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ColumnView {
    pub kind: ColumnKind,
    /// Id do canal (vazio em MASTER e MIC).
    pub id: String,
    pub title: String,
    pub personal: SendView,
    pub transmission: SendView,
}

/// Ordem da spec 9: MASTER primeiro, canais na ordem do perfil, MIC por último.
pub fn columns(profile: &Profile) -> Vec<ColumnView> {
    let factors = profile.chatmix.channels.as_ref().and_then(|(a, b)| {
        chatmix(profile.chatmix.position)
            .ok()
            .map(|f| (a.clone(), b.clone(), f))
    });
    let factor_of = |id: &str| -> Option<f64> {
        factors.as_ref().and_then(|(a, b, (fa, fb))| {
            if id == a {
                Some(*fa)
            } else if id == b {
                Some(*fb)
            } else {
                None
            }
        })
    };
    let mut out = Vec::with_capacity(profile.channels.len() + 2);
    out.push(ColumnView {
        kind: ColumnKind::Master,
        id: String::new(),
        title: "MASTER".into(),
        personal: send_view(&profile.master.personal, None),
        transmission: send_view(&profile.master.transmission, None),
    });
    for c in &profile.channels {
        out.push(ColumnView {
            kind: ColumnKind::Channel,
            id: c.id.clone(),
            title: c.name.clone(),
            personal: send_view(&c.personal, factor_of(&c.id)),
            transmission: send_view(&c.transmission, None),
        });
    }
    let m = &profile.microphone;
    out.push(ColumnView {
        kind: ColumnKind::Mic,
        id: String::new(),
        title: "MIC".into(),
        personal: send_view(&m.personal, None),
        transmission: send_view(&m.transmission, None),
    });
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusKind {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusLine {
    pub kind: StatusKind,
    pub text: String,
}

/// Mensagens de estado a mostrar (spec 9: estados explícitos; cor não é o único indicador, então há sempre texto).
pub fn status_lines(state: &State) -> Vec<StatusLine> {
    let mut lines = Vec::new();
    if !state.connected {
        let text = if state.reconnect_attempts > 0 {
            format!(
                "Sem conexão com o áudio — reconectando (tentativa {})",
                state.reconnect_attempts
            )
        } else {
            "Sem conexão com o áudio".to_owned()
        };
        lines.push(StatusLine {
            kind: StatusKind::Error,
            text,
        });
    }
    if let Some(e) = &state.persist_error {
        lines.push(StatusLine {
            kind: StatusKind::Error,
            text: format!("Alterações não gravadas no disco: {e}"),
        });
    }
    for d in &state.absent_devices {
        lines.push(StatusLine {
            kind: StatusKind::Warning,
            text: format!("Dispositivo ausente: {d}"),
        });
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use iara_core::initial_profile;

    fn state(profile: Profile) -> State {
        State {
            serial: 1,
            profile,
            connected: true,
            persist_error: None,
            absent_devices: vec![],
            reconnect_attempts: 0,
        }
    }

    #[test]
    fn slider_mapping_follows_the_spec_and_round_trips() {
        assert_eq!(position_to_gain(0.0), Gain::SILENCE);
        assert_eq!(position_to_gain(-0.3), Gain::SILENCE);
        assert_eq!(position_to_gain(f64::NAN), Gain::SILENCE);
        assert_eq!(position_to_gain(1.0).db(), Some(0.0));
        assert_eq!(
            position_to_gain(7.0).db(),
            Some(0.0),
            "acima do fim satura em 0 dB"
        );
        assert!((position_to_gain(0.5).db().unwrap() - (-30.0)).abs() < 1e-9);
        for p in [0.001, 0.1, 0.25, 0.9, 1.0] {
            assert!(
                (gain_to_position(position_to_gain(p)) - p).abs() < 1e-9,
                "{p}"
            );
        }
        assert_eq!(gain_to_position(Gain::SILENCE), 0.0);
        // −60 dB exato não pode virar silêncio ao ser mostrado no slider
        let minus_60 = Gain::from_db(-60.0).unwrap();
        assert!(gain_to_position(minus_60) > 0.0);
        assert!(position_to_gain(gain_to_position(minus_60)).db().is_some());
    }

    #[test]
    fn db_labels_use_typographic_minus_and_never_percentages() {
        assert_eq!(format_db(Gain::UNITY), "0,0 dB");
        assert_eq!(format_db(Gain::from_db(-6.0).unwrap()), "−6,0 dB");
        assert_eq!(format_db(Gain::from_db(-59.94).unwrap()), "−59,9 dB");
        assert_eq!(format_db(Gain::SILENCE), "−∞ dB");
        assert!(!format_db(Gain::from_db(-6.0).unwrap()).contains('%'));
    }

    #[test]
    fn columns_are_master_first_channels_in_order_mic_last() {
        let cols = columns(&initial_profile());
        let titles: Vec<_> = cols.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, ["MASTER", "GAME", "CHAT", "MEDIA", "AUX", "MIC"]);
        assert_eq!(cols[0].kind, ColumnKind::Master);
        assert_eq!(cols[5].kind, ColumnKind::Mic);
        assert_eq!(cols[1].id, "game");
        // AUX: sem transmissão por padrão; o estado "desabilitado" chega à interface
        assert!(!cols[4].transmission.enabled && cols[4].personal.enabled);
    }

    #[test]
    fn chatmix_shows_the_extra_attenuation_next_to_the_saved_value_without_changing_it() {
        let mut p = initial_profile();
        p.chatmix.position = 0.5; // favorece CHAT: GAME atenuado em cos(45°) ≈ −3,0 dB
        let cols = columns(&p);
        assert_eq!(cols[1].personal.label, "0,0 dB", "o valor salvo não muda");
        assert_eq!(cols[1].personal.note.as_deref(), Some("ChatMix −3,0 dB"));
        assert_eq!(
            cols[2].personal.note, None,
            "o lado favorecido não é amplificado"
        );
        assert_eq!(
            cols[1].transmission.note, None,
            "transmissão não recebe ChatMix"
        );
        p.chatmix.position = 1.0;
        assert_eq!(
            columns(&p)[1].personal.note.as_deref(),
            Some("ChatMix −∞ dB")
        );
        p.chatmix.channels = None;
        assert!(columns(&p).iter().all(|c| c.personal.note.is_none()));
    }

    #[test]
    fn status_lines_describe_every_problem_in_text() {
        let mut s = state(initial_profile());
        assert!(status_lines(&s).is_empty());
        s.connected = false;
        s.reconnect_attempts = 2;
        s.persist_error = Some("disco cheio".into());
        s.absent_devices = vec!["fone".into()];
        let lines = status_lines(&s);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].kind, StatusKind::Error);
        assert!(lines[0].text.contains("tentativa 2"));
        assert!(lines[1].text.contains("disco cheio"));
        assert_eq!(
            (lines[2].kind, lines[2].text.as_str()),
            (StatusKind::Warning, "Dispositivo ausente: fone")
        );
    }
}
