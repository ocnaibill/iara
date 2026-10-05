use iara_core::Profile;

/// Contrato provisório para a prova técnica; sem API PipeWire no domínio.
/// Resultado aceito pelo backend não equivale a encaminhamento confirmado.
pub trait AudioBackend {
    fn apply_profile(&mut self, desired: &Profile) -> Result<(), BackendError>;
}

#[derive(Debug)]
pub enum BackendError {
    Unavailable,
}

pub struct UnavailableBackend;

impl AudioBackend for UnavailableBackend {
    fn apply_profile(&mut self, _desired: &Profile) -> Result<(), BackendError> {
        Err(BackendError::Unavailable)
    }
}
