//! Valores de padrão do sistema no metadata do PipeWire: `{"name":"<node.name>"}` em `Spa:String:JSON`.

/// Nome do nó dentro de `{"name":"x"}`. Aceita espaços e escapes simples (`\"`, `\\`); `None` se não houver nome.
pub fn parse_name(json: &str) -> Option<String> {
    let after_key = json.split_once("\"name\"")?.1;
    let after_colon = after_key.trim_start().strip_prefix(':')?.trim_start();
    let mut chars = after_colon.strip_prefix('"')?.chars();
    let mut out = String::new();
    loop {
        match chars.next()? {
            '"' => return (!out.is_empty()).then_some(out),
            '\\' => out.push(chars.next()?),
            c => out.push(c),
        }
    }
}

/// `{"name":"x"}` com escape de `\` e `"`.
pub fn name_json(name: &str) -> String {
    format!(
        "{{\"name\":\"{}\"}}",
        name.replace('\\', "\\\\").replace('"', "\\\"")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_through_the_json_the_system_uses() {
        assert_eq!(
            parse_name(r#"{"name":"alsa_output.pci-0000_0b_00.4.analog-stereo"}"#).as_deref(),
            Some("alsa_output.pci-0000_0b_00.4.analog-stereo")
        );
        assert_eq!(
            parse_name(r#"{ "name" : "iara.unassigned" }"#).as_deref(),
            Some("iara.unassigned")
        );
        for n in ["iara.unassigned", "a\"b", "a\\b", "x y"] {
            assert_eq!(parse_name(&name_json(n)).as_deref(), Some(n), "{n}");
        }
        assert_eq!(
            name_json("iara.unassigned"),
            r#"{"name":"iara.unassigned"}"#
        );
    }

    #[test]
    fn malformed_or_empty_values_are_none() {
        for bad in [
            "",
            "{}",
            r#"{"name":""}"#,
            r#"{"name":5}"#,
            r#"{"name":"abc"#,
            "lixo",
            r#"{"nome":"x"}"#,
        ] {
            assert_eq!(parse_name(bad), None, "{bad:?}");
        }
    }
}
