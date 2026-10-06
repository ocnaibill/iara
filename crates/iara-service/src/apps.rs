//! Apresentação e roteamento de aplicativos: junta regras do perfil, escolhas só desta sessão e o que o motor observa.
//! Puro (sem PipeWire e sem tempo): o serviço chama estas funções e a interface só mostra o resultado.

use iara_audio::{AppReport, AppRouteState, RouteTarget, StreamState};
use iara_core::apps::{effective_destination, AppIdentity, Destination, Source};
use iara_core::topology::channel_node;
use iara_core::Profile;
use std::collections::{HashMap, HashSet};

/// Escolhas temporárias desta sessão: chave do aplicativo → `Some(canal)` ou `None` (Não atribuídos).
pub type Overrides = HashMap<String, Option<String>>;

pub use iara_ipc::AppState;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppView {
    pub identity: AppIdentity,
    pub key: Option<String>,
    pub display: String,
    /// Canal efetivo (`None` = Não atribuídos).
    pub channel: Option<String>,
    pub source: Source,
    pub state: AppState,
    pub streams: usize,
}

fn destination_of(
    profile: &Profile,
    overrides: &Overrides,
    app: &AppIdentity,
) -> (Option<String>, Source) {
    match effective_destination(profile, overrides, app) {
        (Destination::Channel(id), src) => (Some(id), src),
        (Destination::Unassigned, src) => (None, src),
    }
}

fn state_of(app: &AppReport) -> AppState {
    match app.state {
        AppRouteState::Applied => AppState::Applied,
        AppRouteState::Applying => AppState::Applying,
        AppRouteState::Partial => AppState::Partial,
        AppRouteState::Unmanaged => AppState::Unmanaged,
        AppRouteState::NotApplied => {
            let not_applied = |s: &&iara_audio::StreamReport| s.state != StreamState::Applied;
            if app
                .streams
                .iter()
                .filter(not_applied)
                .all(|s| s.state == StreamState::DontMove)
            {
                AppState::DontMove
            } else if app
                .streams
                .iter()
                .filter(not_applied)
                .any(|s| s.linked_to.iter().any(|n| n.starts_with("iara.")))
            {
                AppState::Elsewhere
            } else {
                AppState::NotApplied
            }
        }
    }
}

/// Aplicativos para a interface: os que estão tocando (com estado real) e os que têm regra salva mas estão em silêncio.
/// `capture_active`: o Iara é a saída padrão do sistema (aplicativos sem regra deveriam entrar pelo mixer).
pub fn views(
    profile: &Profile,
    overrides: &Overrides,
    reports: &[AppReport],
    capture_active: bool,
) -> Vec<AppView> {
    let mut out: Vec<AppView> = reports
        .iter()
        .map(|r| {
            let (channel, source) = destination_of(profile, overrides, &r.identity);
            let mut state = state_of(r);
            // sem regra, com o Iara como saída padrão, e ainda assim fora do mixer: o aplicativo usa uma saída própria
            if capture_active
                && channel.is_none()
                && source == Source::Default
                && !r.streams.is_empty()
            {
                let at_iara = |s: &iara_audio::StreamReport| {
                    s.linked_to.iter().any(|n| n.starts_with("iara."))
                };
                if r.streams
                    .iter()
                    .all(|s| !at_iara(s) && !s.linked_to.is_empty())
                {
                    state = AppState::Outside;
                }
            }
            AppView {
                key: r.identity.key(),
                display: r.identity.display_name(),
                identity: r.identity.clone(),
                channel,
                source,
                state,
                streams: r.streams.len(),
            }
        })
        .collect();
    let running: Vec<&AppIdentity> = reports.iter().map(|r| &r.identity).collect();
    for rule in &profile.rules {
        if running.iter().any(|a| rule.matcher.matches(a)) {
            continue;
        }
        let identity = AppIdentity {
            app_id: rule.matcher.app_id.clone(),
            binary: rule.matcher.binary.clone(),
            name: rule.matcher.name.clone(),
        };
        let (channel, source) = destination_of(profile, overrides, &identity);
        out.push(AppView {
            key: identity.key(),
            display: identity.display_name(),
            identity,
            channel,
            source,
            state: AppState::Waiting,
            streams: 0,
        });
    }
    out
}

/// Para onde cada aplicativo observado deve ir: canal do Iara ou o roteamento padrão (Não atribuídos).
pub fn route_plan(
    profile: &Profile,
    overrides: &Overrides,
    reports: &[AppReport],
) -> HashMap<String, RouteTarget> {
    reports
        .iter()
        .filter_map(|r| {
            let key = r.identity.key()?;
            let target = match destination_of(profile, overrides, &r.identity).0 {
                Some(id) => RouteTarget::Node(channel_node(&id)),
                None => RouteTarget::Default,
            };
            Some((key, target))
        })
        .collect()
}

/// Escolhas de sessão de aplicativos que não estão mais tocando terminam (spec 8.9). Devolve se algo mudou.
pub fn prune_overrides(overrides: &mut Overrides, reports: &[AppReport]) -> bool {
    let alive: HashSet<String> = reports.iter().filter_map(|r| r.identity.key()).collect();
    let before = overrides.len();
    overrides.retain(|k, _| alive.contains(k));
    overrides.len() != before
}

#[cfg(test)]
mod tests {
    use super::*;
    use iara_audio::StreamReport;
    use iara_core::apps::{AppMatcher, Rule};
    use iara_core::initial_profile;

    fn id(bin: &str) -> AppIdentity {
        AppIdentity {
            binary: Some(bin.into()),
            name: Some(bin.to_uppercase()),
            ..Default::default()
        }
    }

    fn report(bin: &str, state: AppRouteState, streams: &[(StreamState, &[&str])]) -> AppReport {
        AppReport {
            identity: id(bin),
            state,
            streams: streams
                .iter()
                .enumerate()
                .map(|(i, (s, l))| StreamReport {
                    node_id: i as u32,
                    media_name: String::new(),
                    linked_to: l.iter().map(|x| (*x).to_owned()).collect(),
                    state: *s,
                })
                .collect(),
        }
    }

    fn profile_with_rule(bin: &str, ch: &str) -> Profile {
        let mut p = initial_profile();
        p.rules.push(Rule {
            matcher: AppMatcher {
                binary: Some(bin.into()),
                ..Default::default()
            },
            channel: ch.into(),
        });
        p
    }

    #[test]
    fn the_route_plan_follows_session_choice_then_rule_then_default() {
        let p = profile_with_rule("zen", "media");
        let reports = [
            report("zen", AppRouteState::Applied, &[]),
            report("cider", AppRouteState::Unmanaged, &[]),
        ];
        let mut o = Overrides::new();
        let plan = route_plan(&p, &o, &reports);
        assert_eq!(plan["bin:zen"], RouteTarget::Node("iara.ch.media".into()));
        assert_eq!(plan["bin:cider"], RouteTarget::Default);
        o.insert("bin:zen".into(), Some("game".into()));
        assert_eq!(
            route_plan(&p, &o, &reports)["bin:zen"],
            RouteTarget::Node("iara.ch.game".into())
        );
        o.insert("bin:zen".into(), None);
        assert_eq!(
            route_plan(&p, &o, &reports)["bin:zen"],
            RouteTarget::Default
        );
        // aplicativo sem identificação não entra no plano (não se adivinha)
        let anon = AppReport {
            identity: AppIdentity::default(),
            state: AppRouteState::Unmanaged,
            streams: vec![],
        };
        assert!(route_plan(&p, &Overrides::new(), &[anon]).is_empty());
    }

    #[test]
    fn views_show_real_state_and_keep_silent_rules_as_waiting_not_as_refusal() {
        let p = profile_with_rule("zen", "media");
        let mut p2 = p.clone();
        p2.rules.push(Rule {
            matcher: AppMatcher {
                binary: Some("discord".into()),
                ..Default::default()
            },
            channel: "chat".into(),
        });
        let reports = [report(
            "zen",
            AppRouteState::Applied,
            &[(StreamState::Applied, &["iara.ch.media"])],
        )];
        let v = views(&p2, &Overrides::new(), &reports, false);
        assert_eq!(v.len(), 2);
        let zen = v
            .iter()
            .find(|a| a.key.as_deref() == Some("bin:zen"))
            .unwrap();
        assert_eq!(
            (zen.channel.as_deref(), zen.source, zen.state, zen.streams),
            (Some("media"), Source::Rule, AppState::Applied, 1)
        );
        let d = v
            .iter()
            .find(|a| a.key.as_deref() == Some("bin:discord"))
            .unwrap();
        assert_eq!(
            (d.channel.as_deref(), d.state, d.streams),
            (Some("chat"), AppState::Waiting, 0)
        );
        // regra de app que está tocando não duplica a linha
        assert_eq!(views(&p, &Overrides::new(), &reports, false).len(), 1);
    }

    #[test]
    fn not_applied_is_refined_into_dont_move_elsewhere_or_plain_refusal() {
        let p = initial_profile();
        let no = StreamState::NotApplied;
        let s = |r: AppReport| views(&p, &Overrides::new(), &[r], false)[0].state;
        assert_eq!(
            s(report(
                "a",
                AppRouteState::NotApplied,
                &[(StreamState::DontMove, &["fone"])]
            )),
            AppState::DontMove
        );
        assert_eq!(
            s(report(
                "a",
                AppRouteState::NotApplied,
                &[(no, &["iara.ch.chat"])]
            )),
            AppState::Elsewhere,
            "movido por fora para outro canal"
        );
        assert_eq!(
            s(report(
                "a",
                AppRouteState::NotApplied,
                &[(no, &["alsa_output.fone"])]
            )),
            AppState::NotApplied
        );
        assert_eq!(
            s(report(
                "a",
                AppRouteState::Partial,
                &[(StreamState::Applied, &["x"]), (no, &["y"])]
            )),
            AppState::Partial
        );
    }

    #[test]
    fn an_app_without_a_rule_that_keeps_its_own_output_is_outside_the_mixer_only_when_the_iara_is_the_default(
    ) {
        let p = initial_profile();
        let own = report(
            "jogo",
            AppRouteState::Applied,
            &[(StreamState::Applied, &["alsa_output.fone"])],
        );
        let through = report(
            "zen",
            AppRouteState::Applied,
            &[(StreamState::Applied, &["iara.unassigned"])],
        );
        let off = views(
            &p,
            &Overrides::new(),
            &[own.clone(), through.clone()],
            false,
        );
        assert!(
            off.iter().all(|a| a.state != AppState::Outside),
            "sem a saída padrão do Iara, não há expectativa"
        );
        let on = views(&p, &Overrides::new(), &[own, through], true);
        let state = |bin: &str| {
            on.iter()
                .find(|a| a.key.as_deref() == Some(&format!("bin:{bin}")))
                .unwrap()
                .state
        };
        assert_eq!(state("jogo"), AppState::Outside);
        assert_eq!(
            state("zen"),
            AppState::Applied,
            "o que passa pelo Iara não é aviso"
        );
        // com regra, a situação é outra (aplicado ou não), não "fora do mixer"
        let ruled = profile_with_rule("jogo", "game");
        let own = report(
            "jogo",
            AppRouteState::NotApplied,
            &[(StreamState::NotApplied, &["alsa_output.fone"])],
        );
        assert_eq!(
            views(&ruled, &Overrides::new(), &[own], true)[0].state,
            AppState::NotApplied
        );
    }

    #[test]
    fn session_choices_end_when_the_app_stops_playing() {
        let mut o = Overrides::new();
        o.insert("bin:zen".into(), Some("game".into()));
        o.insert("bin:cider".into(), None);
        let reports = [report("zen", AppRouteState::Applied, &[])];
        assert!(prune_overrides(&mut o, &reports));
        assert_eq!(o.keys().collect::<Vec<_>>(), ["bin:zen"]);
        assert!(
            !prune_overrides(&mut o, &reports),
            "nada a podar na segunda vez"
        );
    }
}
