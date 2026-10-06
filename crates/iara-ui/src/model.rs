//! Modelo de apresentação: transforma o retrato do serviço no que a janela mostra e traduz gestos em valores de domínio.
//! Sem GTK e sem D-Bus: tudo aqui é função pura sobre `iara_ipc::State`.

use iara_core::edit::EditCommand;
use iara_core::{chatmix, Gain, Profile, SendControl};
use iara_ipc::{AppEntry, AppSource, AppState, DefaultOutput, ProfileOp, SessionChoice, State};

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

/// Controles extras do MIC, numa área expansível da própria coluna (spec 8.13): mute global sempre visível,
/// ganho de entrada comum e o ramo "Microfone para aplicativos".
#[derive(Debug, Clone, PartialEq)]
pub struct MicExtras {
    pub global_mute: bool,
    pub input_position: f64,
    pub input_label: String,
    pub applications: SendView,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ColumnView {
    pub kind: ColumnKind,
    /// Id do canal (vazio em MASTER e MIC).
    pub id: String,
    pub title: String,
    pub personal: SendView,
    pub transmission: SendView,
    pub mic: Option<MicExtras>,
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
        mic: None,
    });
    for c in &profile.channels {
        out.push(ColumnView {
            kind: ColumnKind::Channel,
            id: c.id.clone(),
            title: c.name.clone(),
            personal: send_view(&c.personal, factor_of(&c.id)),
            transmission: send_view(&c.transmission, None),
            mic: None,
        });
    }
    let m = &profile.microphone;
    out.push(ColumnView {
        kind: ColumnKind::Mic,
        id: String::new(),
        title: "MIC".into(),
        personal: send_view(&m.personal, None),
        transmission: send_view(&m.transmission, None),
        mic: Some(MicExtras {
            global_mute: m.global_mute,
            input_position: gain_to_position(m.input_gain),
            input_label: format_db(m.input_gain),
            applications: send_view(&m.applications, None),
        }),
    });
    out
}

/// Barra do ChatMix: os dois canais do par (pelo nome atual) e a posição em [-1, 1]. `None` com o ChatMix desligado.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatMixView {
    pub first: String,
    pub second: String,
    pub position: f64,
}

pub fn chatmix_view(profile: &Profile) -> Option<ChatMixView> {
    let (a, b) = profile.chatmix.channels.as_ref()?;
    let title = |id: &str| {
        profile
            .channels
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.name.clone())
    };
    Some(ChatMixView {
        first: title(a)?,
        second: title(b)?,
        position: profile.chatmix.position,
    })
}

/// Chave de coalescência: comandos de valor contínuo para o mesmo alvo (um gesto de slider gera dezenas) valem só pelo último.
fn coalesce_key(cmd: &EditCommand) -> Option<String> {
    use EditCommand as E;
    Some(match cmd {
        E::SetChannelGain { channel, send, .. } => format!("cg/{channel}/{send:?}"),
        E::SetMasterGain { send, .. } => format!("mg/{send:?}"),
        E::SetMicInputGain(_) => "mig".to_owned(),
        E::SetMicSendGain { send, .. } => format!("msg/{send:?}"),
        E::SetChatMixPosition(_) => "cmx".to_owned(),
        _ => return None,
    })
}

/// Reduz uma fila de comandos pendentes: dos ajustes de valor para o mesmo alvo fica só o último (na posição do último);
/// o resto (mutes, habilitações, ações estruturais) segue intacto e em ordem.
pub fn coalesce(commands: Vec<EditCommand>) -> Vec<EditCommand> {
    coalesce_by(commands, coalesce_key)
}

/// Igual a `coalesce`, para a fila da janela (edições e escolhas de sessão; estas nunca se fundem).
pub fn coalesce_ui(commands: Vec<UiCommand>) -> Vec<UiCommand> {
    coalesce_by(commands, |c| match c {
        UiCommand::Edit(e) => coalesce_key(e),
        UiCommand::Session { .. } | UiCommand::Deactivate | UiCommand::Profile(_) => None,
    })
}

fn coalesce_by<T>(items: Vec<T>, key: impl Fn(&T) -> Option<String>) -> Vec<T> {
    let mut last: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (i, c) in items.iter().enumerate() {
        if let Some(k) = key(c) {
            last.insert(k, i);
        }
    }
    items
        .into_iter()
        .enumerate()
        .filter(|(i, c)| key(c).is_none_or(|k| last[&k] == *i))
        .map(|(_, c)| c)
        .collect()
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
    match state.default_output {
        DefaultOutput::Released => lines.push(StatusLine {
            kind: StatusKind::Warning,
            text: "A saída padrão do sistema foi trocada: aplicativos novos não passam pelo mixer. \
                   Escolha “Iara — Saída principal” como saída para voltar."
                .to_owned(),
        }),
        DefaultOutput::Waiting if state.connected => lines.push(StatusLine {
            kind: StatusKind::Info,
            text: "O Iara ainda não é a saída padrão do sistema: falta uma saída física preferida presente.".to_owned(),
        }),
        DefaultOutput::Disabled => lines.push(StatusLine {
            kind: StatusKind::Info,
            text: "A captura da saída padrão está desligada na configuração.".to_owned(),
        }),
        _ => {}
    }
    for d in &state.absent_devices {
        lines.push(StatusLine {
            kind: StatusKind::Warning,
            text: format!("Dispositivo ausente: {d}"),
        });
    }
    lines
}

/// Comando que a janela envia ao serviço: edição do perfil ou escolha só desta sessão.
#[derive(Debug, Clone, PartialEq)]
pub enum UiCommand {
    Edit(EditCommand),
    /// “Desligar mixer / voltar ao áudio normal”: restaura a saída padrão anterior e encerra o serviço.
    Deactivate,
    /// Trocar, criar, duplicar, renomear ou excluir perfis.
    Profile(ProfileOp),
    Session {
        key: String,
        choice: SessionChoice,
    },
}

/// Mover um aplicativo para `channel` (`None` = Não atribuídos). Salva uma regra no perfil, ou, com `session_only`,
/// vale só nesta sessão. `None` se o aplicativo não tem identificação utilizável (não se adivinha, spec 7).
pub fn move_app(app: &AppEntry, channel: Option<&str>, session_only: bool) -> Option<UiCommand> {
    if session_only {
        let choice = channel.map_or(SessionChoice::Unassigned, |c| {
            SessionChoice::Channel(c.to_owned())
        });
        return Some(UiCommand::Session {
            key: app.key.clone()?,
            choice,
        });
    }
    Some(UiCommand::Edit(EditCommand::AssignApp {
        matcher: app.identity().suggested_matcher()?,
        channel: channel.map(str::to_owned),
    }))
}

/// "Reaplicar regra": apaga a escolha temporária do aplicativo.
pub fn reapply_rule(app: &AppEntry) -> Option<UiCommand> {
    Some(UiCommand::Session {
        key: app.key.clone()?,
        choice: SessionChoice::Clear,
    })
}

/// Uma opção do seletor de dispositivo (saída ou microfone).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceChoice {
    /// Chave persistente do dispositivo; `None` = "nenhum" (o Iara não liga esse lado).
    pub key: Option<String>,
    pub label: String,
    /// O dispositivo é o preferido do perfil mas não está presente agora (continua registrado; spec 8.5/8.6).
    pub absent: bool,
}

/// Opções do seletor de saída (`output = true`) ou de microfone e qual está escolhida. A primeira é sempre "nenhum".
/// O dispositivo preferido do perfil aparece mesmo ausente, marcado, para a escolha não "sumir" ao desconectar o fone.
pub fn device_choices(state: &State, output: bool) -> (Vec<DeviceChoice>, usize) {
    let preferred = if output {
        state.profile.preferred_output.as_ref()
    } else {
        state.profile.preferred_microphone.as_ref()
    }
    .map(|d| d.persistent_key.clone());
    let none_label = if output {
        "Nenhuma saída (sem áudio na escuta)"
    } else {
        "Nenhum microfone"
    };
    let mut out = vec![DeviceChoice {
        key: None,
        label: none_label.to_owned(),
        absent: false,
    }];
    let mut selected = 0;
    for d in state.devices.iter().filter(|d| d.output == output) {
        if preferred.as_deref() == Some(d.key.as_str()) {
            selected = out.len();
        }
        out.push(DeviceChoice {
            key: Some(d.key.clone()),
            label: d.description.clone(),
            absent: false,
        });
    }
    if let Some(p) = preferred {
        if !out.iter().any(|c| c.key.as_deref() == Some(p.as_str())) {
            selected = out.len();
            out.push(DeviceChoice {
                label: format!("⚠ {p} (ausente)"),
                key: Some(p),
                absent: true,
            });
        }
    }
    (out, selected)
}

/// Linha do menu de perfis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileRow {
    pub id: String,
    pub name: String,
    pub label: String,
    pub active: bool,
    /// Trocar para ele é possível (legível e não é o ativo).
    pub can_switch: bool,
    /// Excluir é possível (não é o ativo e não é o único).
    pub can_delete: bool,
}

/// Texto do botão do cabeçalho: o nome do perfil ativo (o botão já desenha a própria seta).
pub fn active_profile_label(state: &State) -> String {
    state.profile.name.clone()
}

pub fn profile_rows(state: &State) -> Vec<ProfileRow> {
    let active = &state.profile.id;
    let only_one = state.profiles.len() <= 1;
    state
        .profiles
        .iter()
        .map(|p| {
            let is_active = &p.id == active;
            let label = match (is_active, p.readable) {
                (true, _) => format!("● {}", p.name),
                (false, true) => p.name.clone(),
                (false, false) => format!("⚠ {} (ilegível)", p.name),
            };
            ProfileRow {
                id: p.id.clone(),
                name: p.name.clone(),
                label,
                active: is_active,
                can_switch: !is_active && p.readable,
                can_delete: !is_active && !only_one,
            }
        })
        .collect()
}

/// Etiqueta de aplicativo dentro de uma coluna.
#[derive(Debug, Clone, PartialEq)]
pub struct AppChip {
    pub app: AppEntry,
    /// Marca curta ao lado do nome quando o estado pede atenção (vazia se aplicado).
    pub glyph: &'static str,
    pub tooltip: String,
    /// Escolha só desta sessão (borda tracejada na interface).
    pub temporary: bool,
    /// Em silêncio (regra salva, sem fluxo): aparece esmaecido.
    pub idle: bool,
}

pub fn state_text(state: AppState) -> &'static str {
    match state {
        AppState::Applied => "aplicado",
        AppState::Applying => "aplicando",
        AppState::Partial => "aplicado em parte dos fluxos",
        AppState::NotApplied => "não foi possível mover o áudio deste aplicativo",
        AppState::Elsewhere => "está em outro canal (movido por fora; respeitado)",
        AppState::DontMove => "o aplicativo não aceita ser movido",
        AppState::Waiting => "aguardando áudio",
        AppState::Unmanaged => "sem identificação suficiente",
        AppState::Outside => {
            "usa uma saída própria, fora do mixer; associe a um canal para trazê-lo"
        }
    }
}

fn glyph(state: AppState) -> &'static str {
    match state {
        AppState::Applied | AppState::Waiting => "",
        AppState::Applying => "…",
        AppState::Partial => "◐",
        AppState::NotApplied | AppState::DontMove | AppState::Unmanaged => "⚠",
        AppState::Outside => "↗",
        AppState::Elsewhere => "↪",
    }
}

fn chip(app: &AppEntry) -> AppChip {
    let origin = match app.source {
        AppSource::Session => "escolha só desta sessão",
        AppSource::Rule => "regra salva no perfil",
        AppSource::Default => "sem regra",
    };
    let mut tooltip = format!("{} — {} ({origin})", app.display, state_text(app.state));
    if app.state == AppState::NotApplied || app.state == AppState::DontMove {
        tooltip.push_str(
            ". A associação está salva; confira a saída de áudio nas configurações do aplicativo.",
        );
    }
    AppChip {
        glyph: glyph(app.state),
        tooltip,
        temporary: app.source == AppSource::Session,
        idle: app.state == AppState::Waiting,
        app: app.clone(),
    }
}

/// Etiquetas por coluna: a chave é o id do canal; `""` é a coluna MASTER (Não atribuídos). Ordem por nome, estável.
pub fn chips_by_column(state: &State) -> std::collections::HashMap<String, Vec<AppChip>> {
    let mut out: std::collections::HashMap<String, Vec<AppChip>> = std::collections::HashMap::new();
    for a in &state.apps {
        out.entry(a.channel.clone().unwrap_or_default())
            .or_default()
            .push(chip(a));
    }
    for v in out.values_mut() {
        v.sort_by_key(|c| (c.app.display.to_lowercase(), c.app.key.clone()));
    }
    out
}

/// Texto para "não consegui falar com o serviço": o caso comum (serviço parado) em linguagem simples; o resto, com o detalhe.
pub fn unavailable_text(detail: &str) -> String {
    let lower = detail.to_lowercase();
    if lower.contains("not activatable")
        || lower.contains("namehasnoowner")
        || lower.contains("procurando")
    {
        "O serviço do Iara não está em execução. Inicie o iara-service e esta janela se conecta sozinha.".to_owned()
    } else {
        format!("Sem conexão com o serviço do Iara: {detail}")
    }
}

/// Id lógico a partir de um nome digitado (ver `iara_core::slug_id`).
pub fn slug(name: &str, existing: &[String]) -> Option<String> {
    iara_core::slug_id(name, existing)
}

#[cfg(test)]
mod tests {
    use super::*;
    use iara_core::edit::SendKind;
    use iara_core::initial_profile;

    fn state(profile: Profile) -> State {
        State {
            serial: 1,
            profile,
            connected: true,
            persist_error: None,
            absent_devices: vec![],
            reconnect_attempts: 0,
            apps: vec![],
            default_output: DefaultOutput::Active,
            profiles: vec![],
            devices: vec![],
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

    #[test]
    fn mic_column_carries_the_expandable_extras_and_other_columns_do_not() {
        let mut p = initial_profile();
        p.microphone.global_mute = true;
        p.microphone.input_gain = Gain::from_db(-20.0).unwrap();
        let cols = columns(&p);
        let mic = cols.last().unwrap().mic.as_ref().unwrap();
        assert!(mic.global_mute && mic.applications.enabled);
        assert_eq!(mic.input_label, "−20,0 dB");
        assert!((mic.input_position - 2.0 / 3.0).abs() < 1e-9);
        assert!(cols[..cols.len() - 1].iter().all(|c| c.mic.is_none()));
        // o MIC inicial: escuta desabilitada (monitoramento desligado), transmissão ligada
        assert!(
            !cols.last().unwrap().personal.enabled && cols.last().unwrap().transmission.enabled
        );
    }

    #[test]
    fn chatmix_bar_follows_the_pair_by_current_names_and_disappears_when_off() {
        let mut p = initial_profile();
        p.channels[0].name = "Jogos".into();
        p.chatmix.position = -0.4;
        assert_eq!(
            chatmix_view(&p),
            Some(ChatMixView {
                first: "Jogos".into(),
                second: "CHAT".into(),
                position: -0.4
            })
        );
        p.chatmix.channels = None;
        assert_eq!(chatmix_view(&p), None);
    }

    #[test]
    fn a_slider_burst_collapses_to_the_last_value_per_target_and_keeps_everything_else_in_order() {
        let g = |db: f64| Gain::from_db(db).unwrap();
        let gain = |ch: &str, db: f64| EditCommand::SetChannelGain {
            channel: ch.into(),
            send: SendKind::Personal,
            gain: g(db),
        };
        let mute = EditCommand::SetMasterMute {
            send: SendKind::Personal,
            muted: true,
        };
        let queue = vec![
            gain("game", -1.0),
            gain("chat", -2.0),
            gain("game", -3.0),
            mute.clone(),
            gain("game", -4.0),
            EditCommand::SetChatMixPosition(0.1),
            EditCommand::SetChatMixPosition(0.2),
            EditCommand::RemoveChannel {
                channel: "aux".into(),
                destination: None,
            },
        ];
        assert_eq!(
            coalesce(queue),
            vec![
                gain("chat", -2.0),
                mute,
                gain("game", -4.0),
                EditCommand::SetChatMixPosition(0.2),
                EditCommand::RemoveChannel {
                    channel: "aux".into(),
                    destination: None
                },
            ]
        );
        // alvos diferentes (envio de transmissão do mesmo canal) não se fundem
        let tx = EditCommand::SetChannelGain {
            channel: "game".into(),
            send: SendKind::Transmission,
            gain: g(-9.0),
        };
        assert_eq!(coalesce(vec![gain("game", -1.0), tx.clone()]).len(), 2);
    }

    #[test]
    fn slugs_are_safe_unique_ids_from_names() {
        let none: Vec<String> = vec![];
        assert_eq!(slug("Música", &none).as_deref(), Some("musica"));
        assert_eq!(
            slug("  Vídeo  Chamada! ", &none).as_deref(),
            Some("video-chamada")
        );
        assert_eq!(slug("a/b\\c", &none).as_deref(), Some("abc"));
        assert_eq!(slug("!!!", &none), None);
        assert_eq!(slug("", &none), None);
        let taken = vec!["game".to_owned(), "game-2".to_owned()];
        assert_eq!(slug("GAME", &taken).as_deref(), Some("game-3"));
        assert!(iara_core::is_valid_id(
            &slug(&"x".repeat(80), &none).unwrap()
        ));
    }

    #[test]
    fn a_stopped_service_gets_a_plain_message_and_other_failures_keep_their_detail() {
        assert!(
            unavailable_text("serviço indisponível: The name is not activatable")
                .contains("não está em execução")
        );
        assert!(unavailable_text("procurando…").contains("não está em execução"));
        assert!(unavailable_text("barramento sumiu").contains("barramento sumiu"));
    }

    fn entry(
        display: &str,
        key: Option<&str>,
        binary: Option<&str>,
        channel: Option<&str>,
        source: AppSource,
        st: AppState,
    ) -> AppEntry {
        AppEntry {
            key: key.map(Into::into),
            display: display.into(),
            app_id: None,
            binary: binary.map(Into::into),
            name: Some(display.into()),
            channel: channel.map(Into::into),
            source,
            state: st,
            streams: 1,
        }
    }

    #[test]
    fn moving_an_app_saves_a_rule_or_only_a_session_choice_and_never_guesses_an_identity() {
        let zen = entry(
            "Zen",
            Some("bin:zen"),
            Some("zen"),
            None,
            AppSource::Default,
            AppState::Applied,
        );
        match move_app(&zen, Some("media"), false).unwrap() {
            UiCommand::Edit(EditCommand::AssignApp { matcher, channel }) => {
                assert_eq!(
                    (matcher.binary.as_deref(), matcher.name, channel.as_deref()),
                    (Some("zen"), None, Some("media"))
                );
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            move_app(&zen, Some("game"), true),
            Some(UiCommand::Session {
                key: "bin:zen".into(),
                choice: SessionChoice::Channel("game".into())
            })
        );
        assert_eq!(
            move_app(&zen, None, true),
            Some(UiCommand::Session {
                key: "bin:zen".into(),
                choice: SessionChoice::Unassigned
            })
        );
        assert!(matches!(
            move_app(&zen, None, false),
            Some(UiCommand::Edit(EditCommand::AssignApp {
                channel: None,
                ..
            }))
        ));
        assert_eq!(
            reapply_rule(&zen),
            Some(UiCommand::Session {
                key: "bin:zen".into(),
                choice: SessionChoice::Clear
            })
        );
        // sem identificação utilizável: nada a enviar
        let anon = AppEntry {
            key: None,
            binary: None,
            name: None,
            ..zen
        };
        assert_eq!(
            (
                move_app(&anon, Some("game"), false),
                move_app(&anon, Some("game"), true),
                reapply_rule(&anon)
            ),
            (None, None, None)
        );
    }

    #[test]
    fn chips_group_by_channel_sort_by_name_and_describe_problems_in_text() {
        let mut s = state(initial_profile());
        s.apps = vec![
            entry(
                "Zen",
                Some("bin:zen"),
                Some("zen"),
                Some("media"),
                AppSource::Rule,
                AppState::Applied,
            ),
            entry(
                "cider",
                Some("bin:cider"),
                Some("cider"),
                Some("media"),
                AppSource::Session,
                AppState::Elsewhere,
            ),
            entry(
                "Discord",
                Some("bin:discord"),
                Some("discord"),
                Some("chat"),
                AppSource::Rule,
                AppState::Waiting,
            ),
            entry(
                "Jogo",
                Some("bin:jogo"),
                Some("jogo"),
                None,
                AppSource::Default,
                AppState::DontMove,
            ),
        ];
        let by = chips_by_column(&s);
        let media: Vec<_> = by["media"].iter().map(|c| c.app.display.as_str()).collect();
        assert_eq!(
            media,
            ["cider", "Zen"],
            "ordem por nome, sem diferenciar maiúsculas"
        );
        assert!(by["media"][0].temporary && !by["media"][1].temporary);
        assert_eq!(by["media"][0].glyph, "↪");
        assert_eq!(by["media"][1].glyph, "");
        assert!(by["chat"][0].idle && by["chat"][0].tooltip.contains("aguardando áudio"));
        // Não atribuídos fica na coluna "" (MASTER); com recusa, o tooltip orienta sem culpar o aplicativo
        let jogo = &by[""][0];
        assert_eq!(jogo.glyph, "⚠");
        assert!(
            jogo.tooltip.contains("não aceita ser movido")
                && jogo.tooltip.contains("associação está salva")
        );
        // todo estado tem texto (cor nunca é o único indicador)
        for st in [
            AppState::Applied,
            AppState::Applying,
            AppState::Partial,
            AppState::NotApplied,
            AppState::Elsewhere,
            AppState::DontMove,
            AppState::Waiting,
            AppState::Unmanaged,
        ] {
            assert!(!state_text(st).is_empty());
        }
    }

    #[test]
    fn the_ui_queue_collapses_slider_bursts_but_never_merges_session_choices() {
        let g = |db: f64| Gain::from_db(db).unwrap();
        let gain = |db: f64| {
            UiCommand::Edit(EditCommand::SetChannelGain {
                channel: "game".into(),
                send: SendKind::Personal,
                gain: g(db),
            })
        };
        let session = |c: &str| UiCommand::Session {
            key: "bin:zen".into(),
            choice: SessionChoice::Channel(c.into()),
        };
        let out = coalesce_ui(vec![
            gain(-1.0),
            session("game"),
            gain(-2.0),
            session("chat"),
            gain(-3.0),
        ]);
        assert_eq!(out, vec![session("game"), session("chat"), gain(-3.0)]);
    }

    #[test]
    fn the_default_output_situation_is_explained_in_text_and_active_is_quiet() {
        let mut s = state(initial_profile());
        assert!(status_lines(&s).is_empty(), "ativa: nada a dizer");
        s.default_output = DefaultOutput::Released;
        let l = status_lines(&s);
        assert_eq!((l.len(), l[0].kind), (1, StatusKind::Warning));
        assert!(l[0].text.contains("não passam pelo mixer"));
        s.default_output = DefaultOutput::Waiting;
        assert!(status_lines(&s)[0].text.contains("falta uma saída física"));
        s.connected = false; // sem conexão, só o aviso de conexão (não empilhar avisos que dependem dela)
        assert!(!status_lines(&s)
            .iter()
            .any(|l| l.text.contains("falta uma saída")));
        s.connected = true;
        s.default_output = DefaultOutput::Disabled;
        assert!(status_lines(&s)[0].text.contains("desligada"));
    }

    #[test]
    fn an_app_outside_the_mixer_says_so_and_how_to_fix_it() {
        let app = entry(
            "Jogo",
            Some("bin:jogo"),
            Some("jogo"),
            None,
            AppSource::Default,
            AppState::Outside,
        );
        let chip = chip(&app);
        assert_eq!(chip.glyph, "↗");
        assert!(
            chip.tooltip.contains("fora do mixer") && chip.tooltip.contains("associe a um canal")
        );
    }

    #[test]
    fn the_profile_menu_marks_the_active_one_and_only_offers_safe_actions() {
        use iara_ipc::ProfileEntry;
        let mut s = state(initial_profile());
        let pe = |id: &str, name: &str, readable: bool| ProfileEntry {
            id: id.into(),
            name: name.into(),
            readable,
        };
        s.profiles = vec![
            pe("default", "Padrão", true),
            pe("jogos", "Jogos", true),
            pe("quebrado", "quebrado", false),
        ];
        assert_eq!(active_profile_label(&s), "Padrão");
        let rows = profile_rows(&s);
        assert_eq!(rows[0].label, "● Padrão");
        assert_eq!(
            (rows[0].can_switch, rows[0].can_delete),
            (false, false),
            "o ativo não troca nem se exclui"
        );
        assert_eq!((rows[1].can_switch, rows[1].can_delete), (true, true));
        assert_eq!(rows[2].label, "⚠ quebrado (ilegível)");
        assert_eq!(
            (rows[2].can_switch, rows[2].can_delete),
            (false, true),
            "ilegível não vira ativo, mas pode ir para a lixeira"
        );
        // o único perfil não pode ser excluído
        s.profiles = vec![pe("default", "Padrão", true)];
        assert!(!profile_rows(&s)[0].can_delete);
    }

    #[test]
    fn device_choices_list_present_devices_select_the_preferred_and_keep_an_absent_one_visible() {
        use iara_ipc::DeviceEntry;
        let dev = |k: &str, d: &str, o: bool| DeviceEntry {
            key: k.into(),
            description: d.into(),
            output: o,
        };
        let mut s = state(initial_profile());
        s.devices = vec![
            dev("alsa_output.fone", "Fone P2", true),
            dev("alsa_output.hdmi", "Monitor HDMI", true),
            dev("alsa_input.fifine", "fifine AM8", false),
        ];
        // sem preferência: "nenhum" selecionado; saídas e entradas separadas
        let (out, sel) = device_choices(&s, true);
        assert_eq!((out.len(), sel), (3, 0));
        assert_eq!(out[0].key, None);
        assert_eq!(device_choices(&s, false).0.len(), 2);
        // preferido presente: vem selecionado
        s.profile.preferred_output = Some(iara_core::DevicePreference {
            persistent_key: "alsa_output.hdmi".into(),
        });
        let (out, sel) = device_choices(&s, true);
        assert_eq!(out[sel].key.as_deref(), Some("alsa_output.hdmi"));
        assert!(!out[sel].absent);
        // preferido ausente: continua listado e selecionado, marcado — a escolha não some ao desconectar o fone
        s.profile.preferred_microphone = Some(iara_core::DevicePreference {
            persistent_key: "alsa_input.sumiu".into(),
        });
        let (inp, sel) = device_choices(&s, false);
        assert_eq!(inp.len(), 3);
        assert!(
            inp[sel].absent
                && inp[sel].label.starts_with("⚠")
                && inp[sel].label.contains("ausente")
        );
        assert_eq!(inp[sel].key.as_deref(), Some("alsa_input.sumiu"));
    }
}
