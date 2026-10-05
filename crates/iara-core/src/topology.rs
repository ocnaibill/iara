//! Do perfil ao plano de áudio: nós virtuais e ramos (envios) com volume e mute.
//!
//! Puro e sem PipeWire. A topologia espelha as provas da spec 6.3.2: cada canal alimenta o mix pessoal e/ou o de
//! transmissão por ramos independentes; o MASTER age depois da soma; Não atribuídos só chega à escuta; o MIC tem
//! ganho/mute comuns e três ramos (aplicativos, pessoal, transmissão). Envio desabilitado não gera ramo; mute
//! mantém o ramo (preserva ganho e permite desmutar sem recriar).

use crate::{chatmix, is_valid_id, is_valid_text, Profile};
use std::collections::{BTreeMap, BTreeSet};

pub const NODE_PREFIX: &str = "iara.";
pub const UNASSIGNED: &str = "iara.unassigned";
pub const MIX_PERSONAL: &str = "iara.mix.personal";
pub const MIX_TRANSMISSION: &str = "iara.mix.transmission";
pub const MASTER_PERSONAL: &str = "iara.master.personal";
pub const MASTER_TRANSMISSION: &str = "iara.master.transmission";
pub const MIC_INPUT: &str = "iara.mic.input";
pub const MIC_COMMON: &str = "iara.mic.common";
pub const MIC_APPS: &str = "iara.mic.apps";
/// Fontes virtuais selecionáveis por outros aplicativos (OBS, Discord): o MIC dedicado e a transmissão.
/// Ligações com dispositivos físicos: saída pessoal (MASTER pessoal → dispositivo) e entrada do MIC (dispositivo → MIC_INPUT).
pub const DEVICE_OUTPUT: &str = "iara.dev.output";
pub const DEVICE_INPUT: &str = "iara.dev.input";
pub const SOURCE_MIC: &str = "iara.src.mic";
pub const SOURCE_TRANSMISSION: &str = "iara.src.transmission";

pub fn channel_node(id: &str) -> String {
    format!("{NODE_PREFIX}ch.{id}")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeRole {
    Channel,
    Unassigned,
    Mix,
    Master,
    MicInput,
    MicCommon,
    MicApps,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeSpec {
    pub name: String,
    pub role: NodeRole,
    /// Nome apresentado ao usuário (seletores de dispositivo dos aplicativos).
    pub description: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceDirection {
    Output,
    Input,
}

/// Ligação com um dispositivo físico, identificado pela chave persistente (hoje o `node.name`). O ciclo de vida
/// (criar quando presente, remover quando ausente, nunca usar fallback) é do motor; o plano só diz o que se deseja.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceLink {
    pub name: String,
    pub direction: DeviceDirection,
    pub physical: String,
    pub bus: String,
}

/// Fonte virtual (`Audio/Source`) alimentada pelo monitor do nó `from`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceSpec {
    pub name: String,
    pub from: String,
    pub description: String,
}

/// Ramo: copia o sinal de `from` para `to` com `volume` (amplitude linear) e `muted`.
#[derive(Clone, Debug, PartialEq)]
pub struct BranchSpec {
    pub name: String,
    pub from: String,
    pub to: String,
    pub volume: f64,
    pub muted: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub nodes: Vec<NodeSpec>,
    pub branches: Vec<BranchSpec>,
    pub sources: Vec<SourceSpec>,
    pub devices: Vec<DeviceLink>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PlanError {
    InvalidId(String),
    InvalidDeviceKey,
    DuplicateChannel(String),
    UnknownChatMixChannel(String),
    InvalidChatMix,
}

fn branch(from: &str, to: &str, volume: f64, muted: bool) -> BranchSpec {
    let short = |n: &str| n.strip_prefix(NODE_PREFIX).unwrap_or(n).to_owned();
    BranchSpec {
        name: format!("{NODE_PREFIX}br.{}__{}", short(from), short(to)),
        from: from.to_owned(),
        to: to.to_owned(),
        volume,
        muted,
    }
}

pub fn plan(profile: &Profile) -> Result<Plan, PlanError> {
    let mut seen = BTreeSet::new();
    if !is_valid_id(&profile.id) {
        return Err(PlanError::InvalidId(profile.id.clone()));
    }
    for d in [&profile.preferred_output, &profile.preferred_microphone]
        .into_iter()
        .flatten()
    {
        if !is_valid_text(&d.persistent_key, 256) {
            return Err(PlanError::InvalidDeviceKey);
        }
    }
    for c in &profile.channels {
        if !is_valid_id(&c.id) {
            return Err(PlanError::InvalidId(c.id.clone()));
        }
        if !seen.insert(c.id.as_str()) {
            return Err(PlanError::DuplicateChannel(c.id.clone()));
        }
    }
    // ChatMix: multiplicadores extras apenas nos ramos pessoais dos dois canais escolhidos.
    let mut mix_factor: BTreeMap<&str, f64> = BTreeMap::new();
    if let Some((first, second)) = &profile.chatmix.channels {
        for id in [first, second] {
            if !seen.contains(id.as_str()) {
                return Err(PlanError::UnknownChatMixChannel(id.clone()));
            }
        }
        let (a, b) = chatmix(profile.chatmix.position).map_err(|_| PlanError::InvalidChatMix)?;
        mix_factor.insert(first.as_str(), a);
        mix_factor.insert(second.as_str(), b);
    }

    let node = |name: &str, role, description: String| NodeSpec {
        name: name.to_owned(),
        role,
        description,
    };
    let mut nodes = Vec::new();
    for c in &profile.channels {
        nodes.push(node(
            &channel_node(&c.id),
            NodeRole::Channel,
            format!("Iara — {}", c.name),
        ));
    }
    nodes.push(node(
        UNASSIGNED,
        NodeRole::Unassigned,
        "Iara — Saída principal".into(),
    ));
    nodes.push(node(
        MIX_PERSONAL,
        NodeRole::Mix,
        "Iara (interno) — Mix pessoal".into(),
    ));
    nodes.push(node(
        MIX_TRANSMISSION,
        NodeRole::Mix,
        "Iara (interno) — Mix de transmissão".into(),
    ));
    nodes.push(node(
        MASTER_PERSONAL,
        NodeRole::Master,
        "Iara (interno) — MASTER pessoal".into(),
    ));
    nodes.push(node(
        MASTER_TRANSMISSION,
        NodeRole::Master,
        "Iara (interno) — MASTER transmissão".into(),
    ));
    nodes.push(node(
        MIC_INPUT,
        NodeRole::MicInput,
        "Iara (interno) — Entrada do microfone".into(),
    ));
    nodes.push(node(
        MIC_COMMON,
        NodeRole::MicCommon,
        "Iara (interno) — Microfone comum".into(),
    ));
    nodes.push(node(
        MIC_APPS,
        NodeRole::MicApps,
        "Iara (interno) — Microfone para aplicativos".into(),
    ));

    let mut branches = Vec::new();
    for c in &profile.channels {
        let from = channel_node(&c.id);
        if c.personal.enabled {
            let f = mix_factor.get(c.id.as_str()).copied().unwrap_or(1.0);
            branches.push(branch(
                &from,
                MIX_PERSONAL,
                c.personal.gain.amplitude() * f,
                c.personal.muted,
            ));
        }
        if c.transmission.enabled {
            branches.push(branch(
                &from,
                MIX_TRANSMISSION,
                c.transmission.gain.amplitude(),
                c.transmission.muted,
            ));
        }
    }
    // Não atribuídos: só escuta, sem controle próprio (entra pelo MASTER pessoal).
    branches.push(branch(UNASSIGNED, MIX_PERSONAL, 1.0, false));

    let m = &profile.master;
    if m.personal.enabled {
        branches.push(branch(
            MIX_PERSONAL,
            MASTER_PERSONAL,
            m.personal.gain.amplitude(),
            m.personal.muted,
        ));
    }
    if m.transmission.enabled {
        branches.push(branch(
            MIX_TRANSMISSION,
            MASTER_TRANSMISSION,
            m.transmission.gain.amplitude(),
            m.transmission.muted,
        ));
    }

    // MIC: ganho de entrada e mute global antes da divisão; cada ramo com seu ganho/mute. O MASTER não age em MIC_APPS.
    let mic = &profile.microphone;
    branches.push(branch(
        MIC_INPUT,
        MIC_COMMON,
        mic.input_gain.amplitude(),
        mic.global_mute,
    ));
    for (send, to) in [
        (mic.applications, MIC_APPS),
        (mic.personal, MIX_PERSONAL),
        (mic.transmission, MIX_TRANSMISSION),
    ] {
        if send.enabled {
            branches.push(branch(MIC_COMMON, to, send.gain.amplitude(), send.muted));
        }
    }
    let sources = vec![
        SourceSpec {
            name: SOURCE_MIC.into(),
            from: MIC_APPS.into(),
            description: "Iara — Microfone".into(),
        },
        SourceSpec {
            name: SOURCE_TRANSMISSION.into(),
            from: MASTER_TRANSMISSION.into(),
            description: "Iara — Transmissão".into(),
        },
    ];
    let mut devices = Vec::new();
    if let Some(d) = &profile.preferred_output {
        devices.push(DeviceLink {
            name: DEVICE_OUTPUT.into(),
            direction: DeviceDirection::Output,
            physical: d.persistent_key.clone(),
            bus: MASTER_PERSONAL.into(),
        });
    }
    if let Some(d) = &profile.preferred_microphone {
        devices.push(DeviceLink {
            name: DEVICE_INPUT.into(),
            direction: DeviceDirection::Input,
            physical: d.persistent_key.clone(),
            bus: MIC_INPUT.into(),
        });
    }
    Ok(Plan {
        nodes,
        branches,
        sources,
        devices,
    })
}

/// Diferença entre o plano aplicado e o desejado. `retune` só muda volume/mute (via Props); ramos com
/// origem/destino diferentes são removidos e recriados.
#[derive(Debug, Default, PartialEq)]
pub struct PlanDiff<'a> {
    pub add_nodes: Vec<&'a NodeSpec>,
    pub remove_nodes: Vec<&'a NodeSpec>,
    pub add_branches: Vec<&'a BranchSpec>,
    pub remove_branches: Vec<&'a BranchSpec>,
    pub retune: Vec<&'a BranchSpec>,
    pub add_sources: Vec<&'a SourceSpec>,
    pub remove_sources: Vec<&'a SourceSpec>,
}

impl PlanDiff<'_> {
    pub fn is_empty(&self) -> bool {
        self.add_nodes.is_empty()
            && self.remove_nodes.is_empty()
            && self.add_branches.is_empty()
            && self.remove_branches.is_empty()
            && self.retune.is_empty()
            && self.add_sources.is_empty()
            && self.remove_sources.is_empty()
    }
}

/// Diferença entre `current` (aplicado) e `desired`. Os `remove_*` apontam para `current`.
pub fn diff<'a>(current: &'a Plan, desired: &'a Plan) -> PlanDiff<'a> {
    let mut d = PlanDiff::default();
    for n in &desired.nodes {
        if !current.nodes.iter().any(|c| c.name == n.name) {
            d.add_nodes.push(n);
        }
    }
    for n in &current.nodes {
        if !desired.nodes.iter().any(|c| c.name == n.name) {
            d.remove_nodes.push(n);
        }
    }
    for b in &desired.branches {
        match current.branches.iter().find(|c| c.name == b.name) {
            None => d.add_branches.push(b),
            Some(c) if c.from != b.from || c.to != b.to => {
                d.remove_branches.push(c);
                d.add_branches.push(b);
            }
            Some(c) if c.volume != b.volume || c.muted != b.muted => d.retune.push(b),
            Some(_) => {}
        }
    }
    for c in &current.branches {
        if !desired.branches.iter().any(|b| b.name == c.name) {
            d.remove_branches.push(c);
        }
    }
    for src in &desired.sources {
        match current.sources.iter().find(|c| c.name == src.name) {
            None => d.add_sources.push(src),
            Some(c) if c != src => {
                d.remove_sources.push(c);
                d.add_sources.push(src);
            }
            Some(_) => {}
        }
    }
    for c in &current.sources {
        if !desired.sources.iter().any(|s| s.name == c.name) {
            d.remove_sources.push(c);
        }
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{initial_profile, Gain};

    fn find<'a>(p: &'a Plan, from: &str, to: &str) -> Option<&'a BranchSpec> {
        p.branches.iter().find(|b| b.from == from && b.to == to)
    }

    #[test]
    fn initial_profile_has_expected_shape() {
        let p = plan(&initial_profile()).unwrap();
        assert_eq!(p.nodes.len(), 12);
        assert_eq!(p.branches.len(), 13);
        assert_eq!(p.sources.len(), 2);
        // AUX fora da transmissão; Não atribuídos só na escuta; MIC pessoal desabilitado por padrão.
        assert!(find(&p, &channel_node("aux"), MIX_TRANSMISSION).is_none());
        assert!(find(&p, &channel_node("aux"), MIX_PERSONAL).is_some());
        assert!(find(&p, UNASSIGNED, MIX_PERSONAL).is_some());
        assert!(find(&p, UNASSIGNED, MIX_TRANSMISSION).is_none());
        assert!(find(&p, MIC_COMMON, MIX_PERSONAL).is_none());
        assert!(find(&p, MIC_COMMON, MIC_APPS).is_some());
    }

    #[test]
    fn master_never_feeds_mic_apps_and_names_are_unique() {
        let p = plan(&initial_profile()).unwrap();
        assert!(p
            .branches
            .iter()
            .all(|b| b.to != MIC_APPS || b.from == MIC_COMMON));
        let names: BTreeSet<_> = p.branches.iter().map(|b| &b.name).collect();
        assert_eq!(names.len(), p.branches.len());
        assert!(p.nodes.iter().all(|n| n.name.starts_with(NODE_PREFIX)));
    }

    #[test]
    fn mute_keeps_the_branch_and_the_gain_but_disable_removes_it() {
        let mut profile = initial_profile();
        profile.channels[0].personal.gain = Gain::from_db(-6.0).unwrap();
        profile.channels[0].personal.muted = true;
        profile.channels[1].transmission.enabled = false;
        let p = plan(&profile).unwrap();
        let b = find(&p, &channel_node("game"), MIX_PERSONAL).unwrap();
        assert!(b.muted && (b.volume - 0.501_187).abs() < 1e-5);
        assert!(find(&p, &channel_node("chat"), MIX_TRANSMISSION).is_none());
    }

    #[test]
    fn chatmix_only_scales_the_personal_branches_of_the_pair() {
        let mut profile = initial_profile();
        profile.chatmix.position = 1.0; // favorece CHAT: GAME atenuado a zero
        let p = plan(&profile).unwrap();
        assert_eq!(
            find(&p, &channel_node("game"), MIX_PERSONAL)
                .unwrap()
                .volume,
            0.0
        );
        assert_eq!(
            find(&p, &channel_node("chat"), MIX_PERSONAL)
                .unwrap()
                .volume,
            1.0
        );
        assert_eq!(
            find(&p, &channel_node("game"), MIX_TRANSMISSION)
                .unwrap()
                .volume,
            1.0
        );
        assert_eq!(
            find(&p, &channel_node("media"), MIX_PERSONAL)
                .unwrap()
                .volume,
            1.0
        );
    }

    #[test]
    fn mic_common_carries_input_gain_and_global_mute() {
        let mut profile = initial_profile();
        profile.microphone.global_mute = true;
        profile.microphone.input_gain = Gain::from_db(-20.0).unwrap();
        let p = plan(&profile).unwrap();
        let b = find(&p, MIC_INPUT, MIC_COMMON).unwrap();
        assert!(b.muted && (b.volume - 0.1).abs() < 1e-12);
        // os ramos de saída não repetem o ganho comum (evita contar duas vezes)
        assert_eq!(find(&p, MIC_COMMON, MIC_APPS).unwrap().volume, 1.0);
    }

    #[test]
    fn exposed_sources_read_the_right_buses_and_have_readable_names() {
        let p = plan(&initial_profile()).unwrap();
        let mic = p.sources.iter().find(|s| s.name == SOURCE_MIC).unwrap();
        // O microfone dedicado vem de MIC_APPS (fora do MASTER); a transmissão vem depois do MASTER de transmissão.
        assert_eq!(
            (mic.from.as_str(), mic.description.as_str()),
            (MIC_APPS, "Iara — Microfone")
        );
        let tx = p
            .sources
            .iter()
            .find(|s| s.name == SOURCE_TRANSMISSION)
            .unwrap();
        assert_eq!(tx.from, MASTER_TRANSMISSION);
        // Nome legível do canal vem do nome do perfil, não do id.
        let mut profile = initial_profile();
        profile.channels[0].name = "Jogos".into();
        let p = plan(&profile).unwrap();
        let node = p
            .nodes
            .iter()
            .find(|n| n.name == channel_node("game"))
            .unwrap();
        assert_eq!(node.description, "Iara — Jogos");
    }

    #[test]
    fn devices_follow_the_profile_preferences_only() {
        use crate::DevicePreference;
        let mut profile = initial_profile();
        assert!(plan(&profile).unwrap().devices.is_empty());
        profile.preferred_output = Some(DevicePreference {
            persistent_key: "alsa_output.fone".into(),
        });
        profile.preferred_microphone = Some(DevicePreference {
            persistent_key: "alsa_input.mic".into(),
        });
        let p = plan(&profile).unwrap();
        assert_eq!(p.devices.len(), 2);
        let out = p
            .devices
            .iter()
            .find(|d| d.direction == DeviceDirection::Output)
            .unwrap();
        assert_eq!(
            (out.physical.as_str(), out.bus.as_str()),
            ("alsa_output.fone", MASTER_PERSONAL)
        );
        let inp = p
            .devices
            .iter()
            .find(|d| d.direction == DeviceDirection::Input)
            .unwrap();
        assert_eq!(
            (inp.physical.as_str(), inp.bus.as_str()),
            ("alsa_input.mic", MIC_INPUT)
        );
    }

    #[test]
    fn invalid_profiles_are_rejected() {
        let mut profile = initial_profile();
        profile.channels.push(profile.channels[0].clone());
        assert_eq!(
            plan(&profile),
            Err(PlanError::DuplicateChannel("game".into()))
        );
        let mut profile = initial_profile();
        profile.channels[0].id = "x\" }} evil=1 {{ \"".into();
        assert!(matches!(plan(&profile), Err(PlanError::InvalidId(_))));
        let mut profile = initial_profile();
        profile.preferred_output = Some(crate::DevicePreference {
            persistent_key: "a\"b\nc".into(),
        });
        assert_eq!(plan(&profile), Err(PlanError::InvalidDeviceKey));
        let mut profile = initial_profile();
        profile.chatmix.channels = Some(("game".into(), "nao-existe".into()));
        assert_eq!(
            plan(&profile),
            Err(PlanError::UnknownChatMixChannel("nao-existe".into()))
        );
        let mut profile = initial_profile();
        profile.chatmix.position = 2.0;
        assert_eq!(plan(&profile), Err(PlanError::InvalidChatMix));
    }

    #[test]
    fn diff_distinguishes_retune_rebuild_and_node_changes() {
        let base = plan(&initial_profile()).unwrap();
        assert!(diff(&base, &base).is_empty());

        let mut profile = initial_profile();
        profile.channels[0].personal.gain = Gain::from_db(-3.0).unwrap();
        profile.channels[2].transmission.enabled = false;
        profile.microphone.personal.enabled = true;
        let next = plan(&profile).unwrap();
        let d = diff(&base, &next);
        assert_eq!(d.retune.len(), 1);
        assert_eq!(d.remove_branches.len(), 1);
        assert_eq!(d.add_branches.len(), 1);
        assert!(d.add_nodes.is_empty() && d.remove_nodes.is_empty());

        profile.channels.retain(|c| c.id != "aux");
        let smaller = plan(&profile).unwrap();
        let d = diff(&next, &smaller);
        assert_eq!(d.remove_nodes.len(), 1);
        assert_eq!(d.remove_nodes[0].name, channel_node("aux"));
        assert!(d
            .remove_branches
            .iter()
            .any(|b| b.from == channel_node("aux")));
    }
}
