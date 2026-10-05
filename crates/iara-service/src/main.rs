mod backend;

use backend::{AudioBackend, UnavailableBackend};

fn main() -> std::process::ExitCode {
    let profile = iara_core::initial_profile();
    let mut backend = UnavailableBackend;
    match backend.apply_profile(&profile) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Iara: esqueleto de serviço; backend de áudio não implementado: {error:?}");
            std::process::ExitCode::FAILURE
        }
    }
}
