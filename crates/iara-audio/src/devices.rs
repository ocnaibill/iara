//! Inventário de dispositivos físicos de áudio (saídas e entradas) como o Iara os apresenta ao usuário para escolher o
//! fone e o microfone que ele vai rotear (spec 8.5, 8.6, 8.12). Puro: classifica propriedades de nós do PipeWire.

use iara_core::topology::NODE_PREFIX;

/// Dispositivo de áudio selecionável. A chave persistente é o `node.name` (identificação estável em aberto, issue #8).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DeviceInfo {
    /// `true` = saída (sink); `false` = entrada (fonte).
    pub output: bool,
    pub name: String,
    pub description: String,
}

/// Classifica um nó: só `Audio/Sink` e `Audio/Source` que não sejam do próprio Iara. `None` para o resto (fluxos de
/// aplicativos, nós do Iara, vídeo…).
pub fn classify(
    node_name: &str,
    media_class: Option<&str>,
    description: Option<&str>,
    nick: Option<&str>,
    iara_managed: bool,
) -> Option<DeviceInfo> {
    if iara_managed || node_name.starts_with(NODE_PREFIX) {
        return None;
    }
    let output = match media_class? {
        "Audio/Sink" => true,
        "Audio/Source" => false,
        _ => return None,
    };
    let description = [description, nick]
        .into_iter()
        .flatten()
        .find(|d| !d.trim().is_empty())
        .unwrap_or(node_name)
        .trim()
        .to_owned();
    Some(DeviceInfo {
        output,
        name: node_name.to_owned(),
        description,
    })
}

/// Lista estável para a interface: saídas antes das entradas, cada grupo por descrição.
pub fn sorted(mut devices: Vec<DeviceInfo>) -> Vec<DeviceInfo> {
    devices.sort_by(|a, b| {
        (!a.output, a.description.to_lowercase(), &a.name).cmp(&(
            !b.output,
            b.description.to_lowercase(),
            &b.name,
        ))
    });
    devices.dedup();
    devices
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_physical_sinks_and_sources_are_devices() {
        let d = classify(
            "alsa_output.pci-0",
            Some("Audio/Sink"),
            Some("Placa de som"),
            None,
            false,
        )
        .unwrap();
        assert_eq!(
            (d.output, d.name.as_str(), d.description.as_str()),
            (true, "alsa_output.pci-0", "Placa de som")
        );
        let m = classify(
            "alsa_input.usb-fifine",
            Some("Audio/Source"),
            Some("fifine AM8 Pro Mono"),
            None,
            false,
        )
        .unwrap();
        assert!(!m.output);
        // não são dispositivos: nós do Iara (por nome e por marca), fluxos, vídeo, sem classe
        assert_eq!(
            classify(
                "iara.ch.game",
                Some("Audio/Sink"),
                Some("Iara — GAME"),
                None,
                true
            ),
            None
        );
        assert_eq!(
            classify("iara.src.mic", Some("Audio/Source"), None, None, false),
            None
        );
        assert_eq!(
            classify("Zen", Some("Stream/Output/Audio"), None, None, false),
            None
        );
        assert_eq!(
            classify("v4l2_input.x", Some("Video/Source"), None, None, false),
            None
        );
        assert_eq!(classify("sem-classe", None, None, None, false), None);
    }

    #[test]
    fn the_description_falls_back_to_the_nick_then_the_name() {
        let d = |desc, nick| {
            classify("alsa_output.x", Some("Audio/Sink"), desc, nick, false)
                .unwrap()
                .description
        };
        assert_eq!(d(Some("Descrição"), Some("Apelido")), "Descrição");
        assert_eq!(d(Some("  "), Some("Apelido")), "Apelido");
        assert_eq!(d(None, None), "alsa_output.x");
    }

    #[test]
    fn the_list_is_stable_outputs_first_and_without_duplicates() {
        let mk = |o, n: &str, d: &str| DeviceInfo {
            output: o,
            name: n.into(),
            description: d.into(),
        };
        let list = sorted(vec![
            mk(false, "mic-b", "Microfone B"),
            mk(true, "out-z", "zeta"),
            mk(true, "out-a", "Alfa"),
            mk(false, "mic-a", "Microfone A"),
            mk(true, "out-a", "Alfa"),
        ]);
        let names: Vec<_> = list.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["out-a", "out-z", "mic-a", "mic-b"]);
    }
}
