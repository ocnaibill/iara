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

fn split_device_name(s: &str) -> (Option<bool>, &str, Option<&str>) {
    let (is_out, rest) = if let Some(r) = s.strip_prefix("alsa_output.") {
        (Some(true), r)
    } else if let Some(r) = s.strip_prefix("alsa_input.") {
        (Some(false), r)
    } else {
        (None, s)
    };
    if let Some((base, prof)) = rest.rsplit_once('.') {
        (is_out, base, Some(prof))
    } else {
        (is_out, rest, None)
    }
}

fn root_device_id(b: &str) -> &str {
    if let Some((r, _)) = b.rsplit_once('-') {
        r
    } else {
        b
    }
}

/// Verifica se o candidato representa o mesmo hardware físico mesmo se a porta USB ou o caminho
/// do barramento mudou (spec 7, 8.5, issue #8).
fn is_matching_device(desired: &str, candidate_name: &str, candidate_desc: &str) -> bool {
    if desired == candidate_name {
        return true;
    }
    let (d_out, d_base, d_prof) = split_device_name(desired);
    let (c_out, c_base, c_prof) = split_device_name(candidate_name);

    if let (Some(d_o), Some(c_o)) = (d_out, c_out) {
        if d_o != c_o {
            return false;
        }
    }
    if let (Some(dp), Some(cp)) = (d_prof, c_prof) {
        if dp != cp {
            return false;
        }
    }

    if d_base == c_base {
        return true;
    }
    if d_base.starts_with("usb-")
        && c_base.starts_with("usb-")
        && root_device_id(d_base) == root_device_id(c_base)
    {
        return true;
    }
    let d_low = desired.to_lowercase();
    let c_desc_low = candidate_desc.to_lowercase();
    if !c_desc_low.is_empty() && d_low.contains(&c_desc_low) {
        return true;
    }
    false
}

/// Encontra o dispositivo físico presente que corresponde a `desired_physical`.
/// - Se o nome exato estiver presente, retorna-o.
/// - Se não estiver, mas houver exatamente um dispositivo compatível (mesma direção e mesmo hardware),
///   reconecta a ele (tolerante a troca de porta USB).
/// - Se houver múltiplos dispositivos compatíveis (ambiguidade), retorna `None` para exigir seleção explícita.
pub fn resolve_device<'a>(
    desired_physical: &str,
    is_output: bool,
    present_devices: &'a [DeviceInfo],
) -> Option<&'a str> {
    if let Some(d) = present_devices
        .iter()
        .find(|d| d.name == desired_physical && d.output == is_output)
    {
        return Some(&d.name);
    }
    let matches: Vec<&DeviceInfo> = present_devices
        .iter()
        .filter(|d| {
            d.output == is_output
                && is_matching_device(desired_physical, &d.name, &d.description)
        })
        .collect();
    if matches.len() == 1 {
        Some(&matches[0].name)
    } else {
        None
    }
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

    #[test]
    fn resolve_device_handles_exact_match_and_usb_port_changes() {
        let mk = |o, n: &str, d: &str| DeviceInfo {
            output: o,
            name: n.into(),
            description: d.into(),
        };
        let devices = vec![
            mk(
                true,
                "alsa_output.usb-fifine_AM8_Pro_20190808-02.analog-stereo",
                "fifine AM8 Pro Estéreo analógico",
            ),
            mk(
                false,
                "alsa_input.usb-fifine_AM8_Pro_20190808-02.mono-fallback",
                "fifine AM8 Pro Mono",
            ),
            mk(
                true,
                "alsa_output.pci-0000_0b_00.4.analog-stereo",
                "Starship/Matisse HD Audio Controller Estéreo analógico",
            ),
        ];

        // 1) Match exato
        assert_eq!(
            resolve_device(
                "alsa_output.pci-0000_0b_00.4.analog-stereo",
                true,
                &devices
            ),
            Some("alsa_output.pci-0000_0b_00.4.analog-stereo")
        );

        // 2) Dispositivo USB cuja porta mudou (-00 para -02)
        assert_eq!(
            resolve_device(
                "alsa_output.usb-fifine_AM8_Pro_20190808-00.analog-stereo",
                true,
                &devices
            ),
            Some("alsa_output.usb-fifine_AM8_Pro_20190808-02.analog-stereo")
        );

        // 3) Direção errada não casa (entrada vs saída)
        assert_eq!(
            resolve_device(
                "alsa_output.usb-fifine_AM8_Pro_20190808-00.analog-stereo",
                false,
                &devices
            ),
            None
        );

        // 4) Dispositivo ausente
        assert_eq!(
            resolve_device("alsa_output.usb-inexistente.stereo", true, &devices),
            None
        );

        // 5) Ambiguidade: dois dispositivos idênticos exigem escolha explícita
        let ambiguous = vec![
            mk(true, "alsa_output.usb-clone-01.stereo", "Clone"),
            mk(true, "alsa_output.usb-clone-02.stereo", "Clone"),
        ];
        assert_eq!(
            resolve_device("alsa_output.usb-clone-00.stereo", true, &ambiguous),
            None
        );
    }
}
