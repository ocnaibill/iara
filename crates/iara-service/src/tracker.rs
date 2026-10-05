//! Quando gravar e quando registrar histórico (spec 8.14), sem relógio próprio: o tempo entra por parâmetro.
//!
//! - Autosave: grava 300 ms depois do último comando (debounce).
//! - Ajuste contínuo: um gesto vira UMA revisão; ela fecha após 2 s sem novos ajustes e guarda o estado de antes do gesto.
//! - Ação estrutural: revisão própria imediata do estado anterior (fechando antes qualquer gesto aberto).

use iara_core::Profile;
use iara_store::RevisionReason;
use std::time::{Duration, Instant};

pub const AUTOSAVE_DELAY: Duration = Duration::from_millis(300);
pub const GROUP_IDLE: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKind {
    /// Slider ou valor numérico: agrupado.
    Continuous,
    /// Criar/remover/renomear canal, mudar associação, reset, importação aplicada…
    Structural(RevisionReason),
}

#[derive(Debug, PartialEq)]
pub enum Action {
    Save,
    PushRevision(Box<Profile>, RevisionReason),
}

#[derive(Default)]
pub struct EditTracker {
    save_due: Option<Instant>,
    group: Option<(Profile, Instant)>,
}

impl EditTracker {
    /// `previous` é o perfil imediatamente ANTES desta edição.
    pub fn on_edit(&mut self, previous: &Profile, kind: EditKind, now: Instant) -> Vec<Action> {
        let mut out = Vec::new();
        self.save_due = Some(now + AUTOSAVE_DELAY);
        match kind {
            EditKind::Continuous => match &mut self.group {
                Some((_, idle_until)) => *idle_until = now + GROUP_IDLE,
                None => self.group = Some((previous.clone(), now + GROUP_IDLE)),
            },
            EditKind::Structural(reason) => {
                if let Some((base, _)) = self.group.take() {
                    out.push(Action::PushRevision(
                        Box::new(base),
                        RevisionReason::Adjustment,
                    ));
                }
                out.push(Action::PushRevision(Box::new(previous.clone()), reason));
            }
        }
        out
    }

    /// Dispara o que venceu até `now`.
    pub fn on_tick(&mut self, now: Instant) -> Vec<Action> {
        let mut out = Vec::new();
        if self.group.as_ref().is_some_and(|(_, until)| now >= *until) {
            if let Some((base, _)) = self.group.take() {
                out.push(Action::PushRevision(
                    Box::new(base),
                    RevisionReason::Adjustment,
                ));
            }
        }
        if self.save_due.is_some_and(|due| now >= due) {
            self.save_due = None;
            out.push(Action::Save);
        }
        out
    }

    /// Trocar de perfil, desligar ou concluir gesto: grava e fecha o grupo agora.
    pub fn flush(&mut self) -> Vec<Action> {
        let mut out = Vec::new();
        if let Some((base, _)) = self.group.take() {
            out.push(Action::PushRevision(
                Box::new(base),
                RevisionReason::Adjustment,
            ));
        }
        if self.save_due.take().is_some() {
            out.push(Action::Save);
        }
        out
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        [self.save_due, self.group.as_ref().map(|(_, t)| *t)]
            .into_iter()
            .flatten()
            .min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iara_core::{initial_profile, Gain};

    fn ms(base: Instant, n: u64) -> Instant {
        base + Duration::from_millis(n)
    }

    #[test]
    fn a_slider_gesture_makes_one_revision_and_one_save() {
        let t0 = Instant::now();
        let mut tr = EditTracker::default();
        let mut p = initial_profile();
        let original = p.clone();
        // 40 passos de slider, 20 ms entre eles: nada dispara durante o gesto
        for i in 0..40u64 {
            let prev = p.clone();
            p.channels[0].personal.gain = Gain::from_db(-(i as f64)).unwrap();
            assert!(tr
                .on_edit(&prev, EditKind::Continuous, ms(t0, i * 20))
                .is_empty());
            assert!(tr.on_tick(ms(t0, i * 20 + 10)).is_empty());
        }
        let last = 39 * 20;
        // 300 ms depois do último passo: grava, mas ainda não fecha a revisão
        assert_eq!(tr.on_tick(ms(t0, last + 300)), vec![Action::Save]);
        assert!(tr.on_tick(ms(t0, last + 1999)).is_empty());
        // 2 s de inatividade: a revisão guarda o estado de antes do gesto inteiro
        assert_eq!(
            tr.on_tick(ms(t0, last + 2000)),
            vec![Action::PushRevision(
                Box::new(original),
                RevisionReason::Adjustment
            )]
        );
        assert!(tr.next_deadline().is_none());
    }

    #[test]
    fn structural_actions_get_their_own_revision_and_close_the_open_gesture() {
        let t0 = Instant::now();
        let mut tr = EditTracker::default();
        let base = initial_profile();
        let mut p = base.clone();
        let prev = p.clone();
        p.channels[0].personal.gain = Gain::from_db(-3.0).unwrap();
        tr.on_edit(&prev, EditKind::Continuous, t0);
        let before_structure = p.clone();
        p.channels.pop();
        let actions = tr.on_edit(
            &before_structure,
            EditKind::Structural(RevisionReason::Structural),
            ms(t0, 500),
        );
        assert_eq!(
            actions,
            vec![
                Action::PushRevision(Box::new(base), RevisionReason::Adjustment),
                Action::PushRevision(Box::new(before_structure), RevisionReason::Structural),
            ]
        );
        assert_eq!(tr.on_tick(ms(t0, 800)), vec![Action::Save]);
    }

    #[test]
    fn flush_saves_and_closes_everything_now() {
        let t0 = Instant::now();
        let mut tr = EditTracker::default();
        let p = initial_profile();
        tr.on_edit(&p, EditKind::Continuous, t0);
        let actions = tr.flush();
        assert_eq!(actions.len(), 2);
        assert!(matches!(
            actions[0],
            Action::PushRevision(_, RevisionReason::Adjustment)
        ));
        assert_eq!(actions[1], Action::Save);
        assert!(tr.flush().is_empty() && tr.next_deadline().is_none());
    }

    #[test]
    fn next_deadline_is_the_earliest_pending_timer() {
        let t0 = Instant::now();
        let mut tr = EditTracker::default();
        assert!(tr.next_deadline().is_none());
        tr.on_edit(&initial_profile(), EditKind::Continuous, t0);
        assert_eq!(tr.next_deadline(), Some(t0 + AUTOSAVE_DELAY));
        tr.on_tick(t0 + AUTOSAVE_DELAY);
        assert_eq!(tr.next_deadline(), Some(t0 + GROUP_IDLE));
    }
}
