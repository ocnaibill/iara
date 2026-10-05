//! Aplicativos e regras de associação (spec 4, 8.1, 8.2).
//!
//! Um aplicativo é uma origem lógica que pode ter vários fluxos. Sua identidade vem das propriedades que o PipeWire expõe
//! (id do aplicativo, binário, nome); qualquer uma pode faltar. A regra casa por igualdade exata nos campos informados; se
//! vários casam, vence a mais específica e, em empate, a primeira da lista (determinístico e diagnosticável).
//!
//! Precedência de destino (spec 8.2): escolha temporária nesta sessão → regra do perfil → Não atribuídos.

use crate::{is_valid_text, Profile};
use std::collections::HashMap;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppIdentity {
    pub app_id: Option<String>,
    pub binary: Option<String>,
    pub name: Option<String>,
}

impl AppIdentity {
    /// Chave estável para sobreposições de sessão e planos de rota: o campo mais forte disponível, com prefixo.
    /// `None` se o aplicativo não tem nenhuma identificação utilizável (o produto pede seleção em vez de adivinhar).
    pub fn key(&self) -> Option<String> {
        if let Some(v) = &self.app_id {
            Some(format!("id:{v}"))
        } else if let Some(v) = &self.binary {
            Some(format!("bin:{v}"))
        } else {
            self.name.as_ref().map(|v| format!("name:{v}"))
        }
    }

    pub fn display_name(&self) -> String {
        self.name
            .clone()
            .or_else(|| self.binary.clone())
            .or_else(|| self.app_id.clone())
            .unwrap_or_else(|| "(aplicativo sem nome)".to_owned())
    }

    /// Regra mínima que identifica este aplicativo: só o campo mais forte (a associação cobre o aplicativo inteiro).
    pub fn suggested_matcher(&self) -> Option<AppMatcher> {
        let mut m = AppMatcher::default();
        if let Some(v) = &self.app_id {
            m.app_id = Some(v.clone());
        } else if let Some(v) = &self.binary {
            m.binary = Some(v.clone());
        } else {
            m.name = Some(self.name.clone()?);
        }
        Some(m)
    }
}

/// Condição de uma regra: todos os campos presentes precisam ser iguais aos da identidade.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppMatcher {
    pub app_id: Option<String>,
    pub binary: Option<String>,
    pub name: Option<String>,
}

impl AppMatcher {
    pub fn specificity(&self) -> usize {
        [&self.app_id, &self.binary, &self.name]
            .iter()
            .filter(|f| f.is_some())
            .count()
    }

    /// Ao menos um campo e todos textos válidos (perfis importados são entrada não confiável).
    pub fn is_valid(&self) -> bool {
        self.specificity() > 0
            && [&self.app_id, &self.binary, &self.name]
                .into_iter()
                .flatten()
                .all(|v| is_valid_text(v, 256))
    }

    pub fn matches(&self, app: &AppIdentity) -> bool {
        self.specificity() > 0
            && [
                (&self.app_id, &app.app_id),
                (&self.binary, &app.binary),
                (&self.name, &app.name),
            ]
            .iter()
            .all(|(want, have)| want.is_none() || want == have)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub matcher: AppMatcher,
    /// Id do canal de destino.
    pub channel: String,
}

/// A regra que vale para `app`: a mais específica entre as que casam; empate → a primeira.
pub fn resolve<'a>(rules: &'a [Rule], app: &AppIdentity) -> Option<&'a Rule> {
    rules.iter().filter(|r| r.matcher.matches(app)).fold(
        None,
        |best: Option<&Rule>, r| match best {
            Some(b) if b.matcher.specificity() >= r.matcher.specificity() => Some(b),
            _ => Some(r),
        },
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Destination {
    /// Canal do perfil (id).
    Channel(String),
    /// Grupo interno Não atribuídos (audível na escuta, fora da transmissão).
    Unassigned,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Escolha temporária nesta sessão.
    SessionOverride,
    Rule,
    Default,
}

/// Destino efetivo de um aplicativo. `overrides` mapeia a chave do aplicativo a `Some(canal)` ou `None` (Não atribuídos).
/// Um canal que não existe mais no perfil é ignorado (cai para a regra ou para o padrão).
pub fn effective_destination(
    profile: &Profile,
    overrides: &HashMap<String, Option<String>>,
    app: &AppIdentity,
) -> (Destination, Source) {
    let exists = |id: &str| profile.channels.iter().any(|c| c.id == id);
    if let Some(choice) = app.key().and_then(|k| overrides.get(&k)) {
        match choice {
            Some(id) if exists(id) => {
                return (Destination::Channel(id.clone()), Source::SessionOverride)
            }
            None => return (Destination::Unassigned, Source::SessionOverride),
            Some(_) => {}
        }
    }
    match resolve(&profile.rules, app) {
        Some(r) if exists(&r.channel) => (Destination::Channel(r.channel.clone()), Source::Rule),
        _ => (Destination::Unassigned, Source::Default),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::initial_profile;

    fn app(id: Option<&str>, bin: Option<&str>, name: Option<&str>) -> AppIdentity {
        AppIdentity {
            app_id: id.map(Into::into),
            binary: bin.map(Into::into),
            name: name.map(Into::into),
        }
    }

    fn rule(id: Option<&str>, bin: Option<&str>, name: Option<&str>, ch: &str) -> Rule {
        Rule {
            matcher: AppMatcher {
                app_id: id.map(Into::into),
                binary: bin.map(Into::into),
                name: name.map(Into::into),
            },
            channel: ch.into(),
        }
    }

    #[test]
    fn keys_prefer_the_strongest_identification_and_refuse_to_guess() {
        assert_eq!(
            app(Some("org.x.App"), Some("x"), Some("X"))
                .key()
                .as_deref(),
            Some("id:org.x.App")
        );
        assert_eq!(
            app(None, Some("zen"), Some("Zen")).key().as_deref(),
            Some("bin:zen")
        );
        assert_eq!(
            app(None, None, Some("Cider")).key().as_deref(),
            Some("name:Cider")
        );
        assert_eq!(app(None, None, None).key(), None);
        assert_eq!(app(None, None, None).suggested_matcher(), None);
        assert_eq!(app(None, Some("zen"), Some("Zen")).display_name(), "Zen");
        let m = app(None, Some("zen"), Some("Zen"))
            .suggested_matcher()
            .unwrap();
        assert_eq!(
            (m.binary.as_deref(), m.name, m.app_id),
            (Some("zen"), None, None),
            "só o campo mais forte"
        );
    }

    #[test]
    fn a_matcher_needs_every_field_it_names_and_at_least_one() {
        let a = app(Some("id"), Some("bin"), Some("Nome"));
        assert!(AppMatcher {
            binary: Some("bin".into()),
            ..Default::default()
        }
        .matches(&a));
        assert!(AppMatcher {
            binary: Some("bin".into()),
            name: Some("Nome".into()),
            ..Default::default()
        }
        .matches(&a));
        assert!(!AppMatcher {
            binary: Some("bin".into()),
            name: Some("Outro".into()),
            ..Default::default()
        }
        .matches(&a));
        assert!(
            !AppMatcher::default().matches(&a),
            "matcher vazio não casa com tudo"
        );
        assert!(!AppMatcher {
            binary: Some("bin".into()),
            ..Default::default()
        }
        .matches(&app(None, None, Some("Nome"))));
        assert!(!AppMatcher::default().is_valid());
        assert!(!AppMatcher {
            name: Some("a\nb".into()),
            ..Default::default()
        }
        .is_valid());
    }

    #[test]
    fn the_most_specific_rule_wins_and_ties_go_to_the_first() {
        let a = app(None, Some("zen"), Some("Zen"));
        let rules = vec![
            rule(None, None, Some("Zen"), "media"),
            rule(None, Some("zen"), Some("Zen"), "chat"),
            rule(None, Some("zen"), None, "game"),
        ];
        assert_eq!(
            resolve(&rules, &a).unwrap().channel,
            "chat",
            "duas condições vencem uma"
        );
        assert_eq!(resolve(&rules[..1], &a).unwrap().channel, "media");
        let tie = vec![
            rule(None, Some("zen"), None, "game"),
            rule(None, None, Some("Zen"), "media"),
        ];
        assert_eq!(
            resolve(&tie, &a).unwrap().channel,
            "game",
            "empate: a primeira"
        );
        assert!(resolve(&rules, &app(None, Some("outro"), None)).is_none());
    }

    #[test]
    fn precedence_is_session_override_then_rule_then_unassigned() {
        let mut p = initial_profile();
        p.rules = vec![rule(None, Some("zen"), None, "media")];
        let zen = app(None, Some("zen"), Some("Zen"));
        let none = HashMap::new();
        assert_eq!(
            effective_destination(&p, &none, &zen),
            (Destination::Channel("media".into()), Source::Rule)
        );
        let o: HashMap<_, _> = [("bin:zen".to_owned(), Some("game".to_owned()))].into();
        assert_eq!(
            effective_destination(&p, &o, &zen),
            (Destination::Channel("game".into()), Source::SessionOverride)
        );
        let o: HashMap<_, _> = [("bin:zen".to_owned(), None)].into();
        assert_eq!(
            effective_destination(&p, &o, &zen),
            (Destination::Unassigned, Source::SessionOverride)
        );
        // sem regra nem escolha: Não atribuídos
        let cider = app(None, None, Some("Cider"));
        assert_eq!(
            effective_destination(&p, &none, &cider),
            (Destination::Unassigned, Source::Default)
        );
        // canal que deixou de existir é ignorado, sem quebrar
        let o: HashMap<_, _> = [("bin:zen".to_owned(), Some("fantasma".to_owned()))].into();
        assert_eq!(effective_destination(&p, &o, &zen).1, Source::Rule);
        p.rules = vec![rule(None, Some("zen"), None, "fantasma")];
        assert_eq!(
            effective_destination(&p, &none, &zen),
            (Destination::Unassigned, Source::Default)
        );
    }
}
