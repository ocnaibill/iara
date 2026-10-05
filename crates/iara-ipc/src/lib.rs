//! Contrato D-Bus do Iara (barramento de sessão) entre o serviço e as interfaces.
//!
//! - Nome `dev.iara.Mixer`, objeto `/dev/iara/Mixer`, interface `dev.iara.Mixer1`.
//! - A interface nunca é dona do estado: lê um retrato (`GetState`) e muda coisas por comandos granulares; cada comando
//!   devolve a nova versão. O sinal `Changed(serial)` avisa que o retrato mudou (perfil ou status); a interface relê.
//! - Sem samples de áudio no IPC (spec 6.4); medidores ficam para um canal próprio.
//! - Ganho em dB: `-inf` é o silêncio exato; fora disso vale a faixa −60..0.
//! - Este crate não depende do serviço: o servidor fala com um `Controller` abstrato; o cliente só precisa de zbus.

mod client;
mod server;

pub use client::{Client, ClientError, Subscription};
pub use server::{serve, Server};

use iara_core::edit::EditCommand;
use iara_core::Profile;

pub const BUS_NAME: &str = "dev.iara.Mixer";
pub const OBJECT_PATH: &str = "/dev/iara/Mixer";
pub const INTERFACE: &str = "dev.iara.Mixer1";

/// Quem é avisado, com a nova versão, a cada mudança visível do estado.
pub type Notifier = Box<dyn Fn(u64) + Send + Sync>;

/// Retrato do estado, como a interface o vê.
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    pub serial: u64,
    pub profile: Profile,
    pub connected: bool,
    pub persist_error: Option<String>,
    pub absent_devices: Vec<String>,
    pub reconnect_attempts: u32,
}

/// Quem atende o IPC: o serviço. Chamado de threads do D-Bus; deve responder com prazo.
pub trait Controller: Send + Sync + 'static {
    fn state(&self) -> Result<State, String>;
    /// Aplica a edição; devolve a nova versão ou o motivo da recusa.
    fn edit(&self, cmd: EditCommand) -> Result<u64, String>;
}
