//! Posse da saída padrão do sistema (spec 8.2), como máquina de estados pura (sem PipeWire, disco ou relógio).
//!
//! Regras:
//! - Instala “Iara — Saída principal” como padrão só quando há uma saída física preferida presente: instalar sem ela
//!   deixaria o computador inteiro mudo.
//! - Registra o padrão anterior (em disco, via `Effect::Persist`) para restaurar mesmo depois de uma queda.
//! - Se, depois de instalada, o usuário escolher outra saída, o Iara larga a posse: não reinstala e não restaura por cima.
//! - Ao desligar deliberadamente, restaura o anterior apenas se o padrão ainda for o instalado pelo mixer.

use iara_ipc::DefaultOutput;
use iara_store::DefaultSinkState;

/// `node.name` do sink “Iara — Saída principal” (grupo Não atribuídos).
pub const INSTALLED: &str = "iara.unassigned";

#[derive(Clone, Debug, PartialEq, Eq)]
enum Own {
    /// Sem posse: aguardando observação do sistema ou uma saída física presente.
    Idle,
    /// Pedimos a instalação e ainda não vimos o sistema confirmar (não confundir com troca do usuário).
    Pending {
        previous: Option<String>,
    },
    Installed {
        previous: Option<String>,
    },
    /// O usuário trocou a saída depois da instalação.
    Released,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Define (ou, com `None`, apaga) a saída padrão configurada do sistema.
    Set(Option<String>),
    Persist(DefaultSinkState),
    Clear,
}

pub struct Policy {
    enabled: bool,
    own: Own,
    /// Registro lido do disco ao iniciar: de onde vem o “anterior” quando o padrão já é o do Iara (queda e retorno).
    stored: Option<DefaultSinkState>,
}

impl Policy {
    pub fn new(enabled: bool, stored: Option<DefaultSinkState>) -> Self {
        Self {
            enabled,
            own: Own::Idle,
            stored,
        }
    }

    pub fn status(&self) -> DefaultOutput {
        if !self.enabled {
            return DefaultOutput::Disabled;
        }
        match self.own {
            Own::Idle => DefaultOutput::Waiting,
            Own::Pending { .. } | Own::Installed { .. } => DefaultOutput::Active,
            Own::Released => DefaultOutput::Released,
        }
    }

    /// O PipeWire reiniciou: a próxima observação decide de novo (o registro em disco continua valendo).
    pub fn reconnected(&mut self) {
        if matches!(self.own, Own::Pending { .. } | Own::Installed { .. }) {
            if let Own::Pending { previous } | Own::Installed { previous } =
                std::mem::replace(&mut self.own, Own::Idle)
            {
                self.stored = Some(DefaultSinkState::new(INSTALLED, previous));
            }
        }
    }

    /// Reavalia com a última observação da saída padrão configurada e se há uma saída física pronta.
    pub fn evaluate(&mut self, configured: Option<&str>, output_ready: bool) -> Vec<Effect> {
        if !self.enabled {
            return vec![];
        }
        let ours = configured == Some(INSTALLED);
        match self.own.clone() {
            Own::Idle => {
                if ours {
                    // já é nosso (queda e retorno): o anterior vem do registro; nunca o próprio Iara
                    let previous = self
                        .stored
                        .as_ref()
                        .filter(|s| s.installed == INSTALLED)
                        .and_then(|s| s.previous.clone())
                        .filter(|p| p != INSTALLED);
                    self.own = Own::Installed {
                        previous: previous.clone(),
                    };
                    if self.stored.is_none() {
                        return vec![Effect::Persist(DefaultSinkState::new(INSTALLED, previous))];
                    }
                    vec![]
                } else if output_ready {
                    let previous = configured.map(str::to_owned);
                    self.own = Own::Pending {
                        previous: previous.clone(),
                    };
                    vec![
                        Effect::Persist(DefaultSinkState::new(INSTALLED, previous)),
                        Effect::Set(Some(INSTALLED.to_owned())),
                    ]
                } else {
                    vec![]
                }
            }
            Own::Pending { previous } => {
                if ours {
                    self.own = Own::Installed { previous };
                }
                vec![]
            }
            Own::Installed { .. } => {
                if ours {
                    vec![]
                } else {
                    self.own = Own::Released;
                    vec![Effect::Clear]
                }
            }
            Own::Released => {
                if ours {
                    self.own = Own::Installed { previous: None };
                    vec![Effect::Persist(DefaultSinkState::new(INSTALLED, None))]
                } else {
                    vec![]
                }
            }
        }
    }

    /// “Desligar mixer / voltar ao áudio normal”: restaura o anterior se (e só se) o padrão ainda for o nosso.
    pub fn deactivate(&mut self, configured: Option<&str>) -> Vec<Effect> {
        let previous = match std::mem::replace(&mut self.own, Own::Idle) {
            Own::Pending { previous } | Own::Installed { previous } => previous,
            Own::Idle | Own::Released => return vec![],
        };
        let mut out = Vec::new();
        // `Pending` ainda sem confirmação conta como nosso; observado outro valor, o usuário já escolheu: não mexer
        if configured == Some(INSTALLED)
            || configured == previous.as_deref()
            || configured.is_none()
        {
            out.push(Effect::Set(previous.filter(|p| p != INSTALLED)));
        }
        out.push(Effect::Clear);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FONE: &str = "alsa_output.pci-0000_0b_00.4.analog-stereo";
    const CAIXAS: &str = "alsa_output.usb-caixas";

    fn st(previous: Option<&str>) -> DefaultSinkState {
        DefaultSinkState::new(INSTALLED, previous.map(Into::into))
    }

    #[test]
    fn it_installs_only_with_a_ready_output_and_records_the_previous_default_first() {
        let mut p = Policy::new(true, None);
        assert_eq!(p.status(), DefaultOutput::Waiting);
        // sem saída física pronta: não instala (o sistema inteiro ficaria mudo)
        assert!(p.evaluate(Some(FONE), false).is_empty());
        assert_eq!(p.status(), DefaultOutput::Waiting);
        // pronta: grava o anterior ANTES de trocar, e só então define
        let fx = p.evaluate(Some(FONE), true);
        assert_eq!(
            fx,
            vec![
                Effect::Persist(st(Some(FONE))),
                Effect::Set(Some(INSTALLED.into()))
            ]
        );
        assert_eq!(p.status(), DefaultOutput::Active);
        // sem padrão configurado antes: anterior é "nenhum"
        let mut q = Policy::new(true, None);
        assert_eq!(q.evaluate(None, true)[0], Effect::Persist(st(None)));
    }

    #[test]
    fn a_stale_observation_right_after_installing_is_not_mistaken_for_a_user_change() {
        let mut p = Policy::new(true, None);
        p.evaluate(Some(FONE), true);
        // o sistema ainda reporta o valor antigo por um instante: continua pendente, sem largar a posse
        assert!(p.evaluate(Some(FONE), true).is_empty());
        assert_eq!(p.status(), DefaultOutput::Active);
        // confirmação
        assert!(p.evaluate(Some(INSTALLED), true).is_empty());
        // agora uma troca de verdade
        assert_eq!(p.evaluate(Some(CAIXAS), true), vec![Effect::Clear]);
        assert_eq!(p.status(), DefaultOutput::Released);
    }

    #[test]
    fn a_user_choice_after_installing_is_respected_never_reinstalled_nor_restored_over() {
        let mut p = Policy::new(true, None);
        p.evaluate(Some(FONE), true);
        p.evaluate(Some(INSTALLED), true);
        assert_eq!(p.evaluate(Some(CAIXAS), true), vec![Effect::Clear]);
        // continua largado, mesmo com saída pronta e passando o tempo
        for _ in 0..3 {
            assert!(p.evaluate(Some(CAIXAS), true).is_empty());
        }
        // desligar não restaura nada por cima da escolha do usuário
        assert!(p.deactivate(Some(CAIXAS)).is_empty());
        // se o usuário escolher o Iara de novo, a posse volta (sem anterior conhecido)
        let mut q = Policy::new(true, None);
        q.evaluate(Some(FONE), true);
        q.evaluate(Some(INSTALLED), true);
        q.evaluate(Some(CAIXAS), true);
        assert_eq!(
            q.evaluate(Some(INSTALLED), true),
            vec![Effect::Persist(st(None))]
        );
        assert_eq!(q.status(), DefaultOutput::Active);
    }

    #[test]
    fn deactivating_restores_the_previous_default_only_while_it_is_still_ours() {
        let mut p = Policy::new(true, None);
        p.evaluate(Some(FONE), true);
        p.evaluate(Some(INSTALLED), true);
        assert_eq!(
            p.deactivate(Some(INSTALLED)),
            vec![Effect::Set(Some(FONE.into())), Effect::Clear]
        );
        assert_eq!(p.status(), DefaultOutput::Waiting);
        // sem padrão anterior configurado: restaurar = apagar a escolha
        let mut q = Policy::new(true, None);
        q.evaluate(None, true);
        q.evaluate(Some(INSTALLED), true);
        assert_eq!(
            q.deactivate(Some(INSTALLED)),
            vec![Effect::Set(None), Effect::Clear]
        );
        // desligar antes de o sistema confirmar a instalação (pendente) também desfaz
        let mut r = Policy::new(true, None);
        r.evaluate(Some(FONE), true);
        assert_eq!(
            r.deactivate(Some(FONE)),
            vec![Effect::Set(Some(FONE.into())), Effect::Clear]
        );
        // sem posse: nada a fazer
        assert!(Policy::new(true, None).deactivate(Some(FONE)).is_empty());
    }

    #[test]
    fn after_a_crash_the_previous_default_comes_from_disk_and_is_never_the_iara_itself() {
        // o serviço caiu com o Iara como padrão e voltou: o sistema já reporta o Iara
        let mut p = Policy::new(true, Some(st(Some(FONE))));
        assert!(
            p.evaluate(Some(INSTALLED), false).is_empty(),
            "já é nosso: nada a instalar"
        );
        assert_eq!(p.status(), DefaultOutput::Active);
        assert_eq!(
            p.deactivate(Some(INSTALLED)),
            vec![Effect::Set(Some(FONE.into())), Effect::Clear]
        );
        // registro ausente ou corrompido: restaurar apaga a escolha (nunca "restaura" o próprio Iara)
        let mut q = Policy::new(true, None);
        assert_eq!(
            q.evaluate(Some(INSTALLED), false),
            vec![Effect::Persist(st(None))]
        );
        assert_eq!(
            q.deactivate(Some(INSTALLED)),
            vec![Effect::Set(None), Effect::Clear]
        );
        // registro que aponta o anterior como o próprio Iara é ignorado
        let mut r = Policy::new(true, Some(st(Some(INSTALLED))));
        r.evaluate(Some(INSTALLED), false);
        assert_eq!(
            r.deactivate(Some(INSTALLED)),
            vec![Effect::Set(None), Effect::Clear]
        );
    }

    #[test]
    fn a_pipewire_restart_re_decides_from_the_disk_record() {
        let mut p = Policy::new(true, None);
        p.evaluate(Some(FONE), true);
        p.evaluate(Some(INSTALLED), true);
        p.reconnected();
        assert_eq!(p.status(), DefaultOutput::Waiting);
        // o WirePlumber restaurou a configuração do Iara: reconhece como nosso e preserva o anterior
        assert!(p.evaluate(Some(INSTALLED), false).is_empty());
        assert_eq!(
            p.deactivate(Some(INSTALLED)),
            vec![Effect::Set(Some(FONE.into())), Effect::Clear]
        );
    }

    #[test]
    fn disabled_means_hands_off() {
        let mut p = Policy::new(false, None);
        assert_eq!(p.status(), DefaultOutput::Disabled);
        assert!(p.evaluate(Some(FONE), true).is_empty());
        assert!(p.deactivate(Some(INSTALLED)).is_empty());
    }
}
