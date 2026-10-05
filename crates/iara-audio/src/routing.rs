//! Decisão de roteamento de fluxos de aplicativos, pura (sem PipeWire): o que fazer com cada fluxo e em que estado ele está.
//!
//! Regras de conduta (spec 8.1, 8.9, 8.11):
//! - Move-se um fluxo no máximo uma vez por destino desejado; depois disso não se briga: mudanças externas são respeitadas
//!   (a interface mostra o fluxo "em outro destino"). Só uma nova decisão (outro destino desejado) volta a mover.
//! - Fluxo com `node.dont-move` nunca é movido: é reportado como não aplicado, com o motivo.
//! - "Aplicado" só vale quando o link real do fluxo vai ao destino desejado; um comando aceito não prova nada.
//! - Sem destino de rota para o aplicativo (`RouteTarget::Default`), o roteamento padrão do sistema vale; se o Iara tinha
//!   movido o fluxo antes, a sobreposição é removida.

use iara_core::apps::AppIdentity;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

/// Tempo para o link aparecer depois de pedir a mudança; passado isso sem efeito, o fluxo conta como não aplicado.
pub const APPLY_GRACE: Duration = Duration::from_secs(3);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouteTarget {
    /// `node.name` do sink de destino (canal do Iara).
    Node(String),
    /// Roteamento padrão do sistema (Não atribuídos).
    Default,
}

/// O que se observa de um fluxo de reprodução.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamObs {
    pub id: u32,
    pub app: AppIdentity,
    pub media_name: String,
    pub dont_move: bool,
    /// `node.name` dos sinks aos quais o fluxo está ligado agora.
    pub linked_to: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamState {
    /// Link real no destino desejado (ou no padrão do sistema quando é isso que se deseja).
    Applied,
    /// Mudança pedida há menos de `APPLY_GRACE`, ou destino ainda não existe.
    Applying,
    /// O fluxo não aceita ser movido (`node.dont-move`).
    DontMove,
    /// Pedimos a mudança e, passado o prazo, o fluxo não foi para o destino (recusa, ou alguém o moveu depois).
    NotApplied,
    /// O aplicativo não tem rota no plano (sem identificação utilizável ou plano ainda não definido).
    Unmanaged,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamReport {
    pub node_id: u32,
    pub media_name: String,
    pub linked_to: Vec<String>,
    pub state: StreamState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppRouteState {
    Applied,
    Applying,
    /// Parte dos fluxos aplicada, parte não.
    Partial,
    NotApplied,
    Unmanaged,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppReport {
    pub identity: AppIdentity,
    pub streams: Vec<StreamReport>,
    pub state: AppRouteState,
}

/// Ação a executar sobre um fluxo. `send = false` só registra que o fluxo já está no destino (para poder devolvê-lo ao padrão depois).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteAction {
    pub stream: u32,
    /// `Some(nó)` define o destino por metadata; `None` remove a sobreposição (volta ao padrão).
    pub target: Option<String>,
    pub send: bool,
}

/// Tentativas já feitas: fluxo → (destino pedido, instante). `None` = pedimos o padrão do sistema.
pub type Attempts = HashMap<u32, (Option<String>, Instant)>;

pub fn pending_actions(
    streams: &[StreamObs],
    routes: &HashMap<String, RouteTarget>,
    attempts: &Attempts,
    present_nodes: &HashSet<String>,
) -> Vec<RouteAction> {
    let mut out = Vec::new();
    for s in streams {
        let Some(route) = s.app.key().and_then(|k| routes.get(&k)) else {
            continue;
        };
        let tried = attempts.get(&s.id).map(|(t, _)| t);
        match route {
            RouteTarget::Node(t) => {
                if s.linked_to.contains(t) {
                    if tried != Some(&Some(t.clone())) {
                        out.push(RouteAction {
                            stream: s.id,
                            target: Some(t.clone()),
                            send: false,
                        });
                    }
                } else if !s.dont_move
                    && present_nodes.contains(t)
                    && tried != Some(&Some(t.clone()))
                {
                    out.push(RouteAction {
                        stream: s.id,
                        target: Some(t.clone()),
                        send: true,
                    });
                }
            }
            RouteTarget::Default => {
                if matches!(tried, Some(Some(_))) {
                    out.push(RouteAction {
                        stream: s.id,
                        target: None,
                        send: true,
                    });
                }
            }
        }
    }
    out
}

fn stream_state(
    s: &StreamObs,
    route: Option<&RouteTarget>,
    attempt: Option<&(Option<String>, Instant)>,
    now: Instant,
) -> StreamState {
    let Some(route) = route else {
        return StreamState::Unmanaged;
    };
    match route {
        RouteTarget::Node(t) => {
            if s.linked_to.contains(t) {
                StreamState::Applied
            } else if s.dont_move {
                StreamState::DontMove
            } else {
                match attempt {
                    Some((Some(want), at))
                        if want == t && now.duration_since(*at) >= APPLY_GRACE =>
                    {
                        StreamState::NotApplied
                    }
                    _ => StreamState::Applying,
                }
            }
        }
        // sem destino próprio: o padrão do sistema vale; só "aplicando" enquanto a sobreposição antiga é removida
        RouteTarget::Default => match attempt {
            Some((None, at)) if now.duration_since(*at) < APPLY_GRACE => StreamState::Applying,
            _ => StreamState::Applied,
        },
    }
}

/// Estado por aplicativo (fluxos agrupados pela identidade), ordenado por nome para a interface não pular.
pub fn report(
    streams: &[StreamObs],
    routes: &HashMap<String, RouteTarget>,
    attempts: &Attempts,
    now: Instant,
) -> Vec<AppReport> {
    let mut groups: Vec<(AppIdentity, Vec<StreamReport>)> = Vec::new();
    let mut sorted: Vec<&StreamObs> = streams.iter().collect();
    sorted.sort_by_key(|s| s.id);
    for s in sorted {
        let route = s.app.key().and_then(|k| routes.get(&k));
        let state = stream_state(s, route, attempts.get(&s.id), now);
        let rep = StreamReport {
            node_id: s.id,
            media_name: s.media_name.clone(),
            linked_to: s.linked_to.clone(),
            state,
        };
        match groups
            .iter_mut()
            .find(|(a, _)| a.key().is_some() && a.key() == s.app.key() || *a == s.app)
        {
            Some((_, v)) => v.push(rep),
            None => groups.push((s.app.clone(), vec![rep])),
        }
    }
    let mut apps: Vec<AppReport> = groups
        .into_iter()
        .map(|(identity, streams)| {
            let all = |p: fn(StreamState) -> bool| streams.iter().all(|s| p(s.state));
            let any = |p: fn(StreamState) -> bool| streams.iter().any(|s| p(s.state));
            let state = if all(|s| s == StreamState::Unmanaged) {
                AppRouteState::Unmanaged
            } else if all(|s| s == StreamState::Applied) {
                AppRouteState::Applied
            } else if any(|s| s == StreamState::Applying)
                && !any(|s| matches!(s, StreamState::NotApplied | StreamState::DontMove))
            {
                AppRouteState::Applying
            } else if any(|s| s == StreamState::Applied) {
                AppRouteState::Partial
            } else {
                AppRouteState::NotApplied
            };
            AppReport {
                identity,
                streams,
                state,
            }
        })
        .collect();
    apps.sort_by_key(|a| (a.identity.display_name().to_lowercase(), a.identity.key()));
    apps
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(bin: &str) -> AppIdentity {
        AppIdentity {
            binary: Some(bin.into()),
            name: Some(bin.to_uppercase()),
            ..Default::default()
        }
    }

    fn stream(id: u32, bin: &str, linked: &[&str]) -> StreamObs {
        StreamObs {
            id,
            app: app(bin),
            media_name: format!("s{id}"),
            dont_move: false,
            linked_to: linked.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    fn routes(pairs: &[(&str, RouteTarget)]) -> HashMap<String, RouteTarget> {
        pairs
            .iter()
            .map(|(b, t)| (format!("bin:{b}"), t.clone()))
            .collect()
    }

    fn node(n: &str) -> RouteTarget {
        RouteTarget::Node(n.into())
    }

    fn present(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn a_stream_is_moved_once_to_its_target_and_never_fought_afterwards() {
        let t0 = Instant::now();
        let r = routes(&[("zen", node("iara.ch.media"))]);
        let pres = present(&["iara.ch.media"]);
        let mut attempts = Attempts::new();
        let s = vec![stream(7, "zen", &["alsa_output.fone"])];
        let acts = pending_actions(&s, &r, &attempts, &pres);
        assert_eq!(
            acts,
            vec![RouteAction {
                stream: 7,
                target: Some("iara.ch.media".into()),
                send: true
            }]
        );
        attempts.insert(7, (Some("iara.ch.media".into()), t0));
        // o fluxo ficou no mesmo lugar (recusa): não repete o comando
        assert!(pending_actions(&s, &r, &attempts, &pres).is_empty());
        // alguém o moveu para outro lugar depois de aplicado: continua sem brigar
        let moved = vec![stream(7, "zen", &["alsa_output.caixas"])];
        assert!(pending_actions(&moved, &r, &attempts, &pres).is_empty());
        // nova decisão do usuário (outro canal): volta a mover
        let r2 = routes(&[("zen", node("iara.ch.game"))]);
        let pres2 = present(&["iara.ch.media", "iara.ch.game"]);
        assert_eq!(
            pending_actions(&moved, &r2, &attempts, &pres2),
            vec![RouteAction {
                stream: 7,
                target: Some("iara.ch.game".into()),
                send: true
            }]
        );
    }

    #[test]
    fn nothing_is_sent_for_dont_move_streams_or_targets_that_do_not_exist_yet() {
        let r = routes(&[("zen", node("iara.ch.media"))]);
        let attempts = Attempts::new();
        let mut s = stream(7, "zen", &["alsa_output.fone"]);
        s.dont_move = true;
        assert!(pending_actions(&[s], &r, &attempts, &present(&["iara.ch.media"])).is_empty());
        assert!(
            pending_actions(&[stream(8, "zen", &[])], &r, &attempts, &present(&[])).is_empty(),
            "destino ainda não existe"
        );
        // aplicativo sem rota no plano: ignorado
        assert!(pending_actions(
            &[stream(9, "outro", &[])],
            &r,
            &attempts,
            &present(&["iara.ch.media"])
        )
        .is_empty());
    }

    #[test]
    fn a_stream_already_at_the_target_is_adopted_so_it_can_be_returned_to_default_later() {
        let t0 = Instant::now();
        let r = routes(&[("zen", node("iara.ch.media"))]);
        let pres = present(&["iara.ch.media"]);
        let s = vec![stream(7, "zen", &["iara.ch.media"])];
        let acts = pending_actions(&s, &r, &Attempts::new(), &pres);
        assert_eq!(
            acts,
            vec![RouteAction {
                stream: 7,
                target: Some("iara.ch.media".into()),
                send: false
            }]
        );
        let mut attempts = Attempts::new();
        attempts.insert(7, (Some("iara.ch.media".into()), t0));
        assert!(
            pending_actions(&s, &r, &attempts, &pres).is_empty(),
            "adotado: nada mais a fazer"
        );
        // desassociar: a sobreposição é removida (volta ao padrão)
        let unassign = routes(&[("zen", RouteTarget::Default)]);
        assert_eq!(
            pending_actions(&s, &unassign, &attempts, &pres),
            vec![RouteAction {
                stream: 7,
                target: None,
                send: true
            }]
        );
        // e uma vez devolvido ao padrão, não repete
        attempts.insert(7, (None, t0));
        assert!(pending_actions(&s, &unassign, &attempts, &pres).is_empty());
        // um fluxo que o Iara nunca moveu não é tocado ao "desassociar"
        assert!(pending_actions(&s, &unassign, &Attempts::new(), &pres).is_empty());
    }

    #[test]
    fn states_report_what_the_real_links_say_not_what_was_asked() {
        let t0 = Instant::now();
        let r = routes(&[("zen", node("iara.ch.media"))]);
        let mut attempts = Attempts::new();
        // sem tentativa ainda: aplicando
        let rep = report(&[stream(7, "zen", &["fone"])], &r, &attempts, t0);
        assert_eq!(
            (rep[0].state, rep[0].streams[0].state),
            (AppRouteState::Applying, StreamState::Applying)
        );
        attempts.insert(7, (Some("iara.ch.media".into()), t0));
        // dentro do prazo: ainda aplicando
        let rep = report(
            &[stream(7, "zen", &["fone"])],
            &r,
            &attempts,
            t0 + Duration::from_secs(1),
        );
        assert_eq!(rep[0].state, AppRouteState::Applying);
        // passado o prazo sem o link: não aplicado (comando aceito não prova nada)
        let rep = report(
            &[stream(7, "zen", &["fone"])],
            &r,
            &attempts,
            t0 + APPLY_GRACE,
        );
        assert_eq!(
            (rep[0].state, rep[0].streams[0].state),
            (AppRouteState::NotApplied, StreamState::NotApplied)
        );
        // link no destino: aplicado
        let rep = report(
            &[stream(7, "zen", &["iara.ch.media"])],
            &r,
            &attempts,
            t0 + APPLY_GRACE,
        );
        assert_eq!(rep[0].state, AppRouteState::Applied);
        // dont-move: motivo próprio, sem esperar prazo
        let mut dm = stream(8, "zen", &["fone"]);
        dm.dont_move = true;
        let rep = report(&[dm], &r, &Attempts::new(), t0);
        assert_eq!(rep[0].streams[0].state, StreamState::DontMove);
        // sem rota: não gerenciado
        let rep = report(&[stream(9, "x", &[])], &r, &attempts, t0);
        assert_eq!(rep[0].state, AppRouteState::Unmanaged);
    }

    #[test]
    fn streams_of_one_app_are_grouped_and_a_mixed_result_is_partial() {
        let t0 = Instant::now();
        let r = routes(&[("zen", node("iara.ch.media"))]);
        let mut attempts = Attempts::new();
        attempts.insert(1, (Some("iara.ch.media".into()), t0));
        attempts.insert(2, (Some("iara.ch.media".into()), t0));
        let s = vec![
            stream(2, "zen", &["fone"]),
            stream(1, "zen", &["iara.ch.media"]),
            stream(3, "cider", &["fone"]),
        ];
        let rep = report(&s, &r, &attempts, t0 + APPLY_GRACE);
        assert_eq!(rep.len(), 2);
        let zen = rep
            .iter()
            .find(|a| a.identity.binary.as_deref() == Some("zen"))
            .unwrap();
        assert_eq!(
            zen.streams.iter().map(|x| x.node_id).collect::<Vec<_>>(),
            [1, 2],
            "fluxos em ordem estável"
        );
        assert_eq!(zen.state, AppRouteState::Partial);
        // ordem por nome para a interface não pular
        assert_eq!(rep[0].identity.display_name(), "CIDER");
    }

    #[test]
    fn default_route_reports_applied_and_applying_only_while_the_old_override_is_removed() {
        let t0 = Instant::now();
        let r = routes(&[("zen", RouteTarget::Default)]);
        let mut attempts = Attempts::new();
        assert_eq!(
            report(&[stream(7, "zen", &["fone"])], &r, &attempts, t0)[0].state,
            AppRouteState::Applied
        );
        attempts.insert(7, (None, t0));
        assert_eq!(
            report(&[stream(7, "zen", &["fone"])], &r, &attempts, t0)[0].state,
            AppRouteState::Applying
        );
        assert_eq!(
            report(
                &[stream(7, "zen", &["fone"])],
                &r,
                &attempts,
                t0 + APPLY_GRACE
            )[0]
            .state,
            AppRouteState::Applied
        );
    }
}
