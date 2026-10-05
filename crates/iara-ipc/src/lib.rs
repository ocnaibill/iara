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

use serde::{Deserialize, Serialize};

use iara_core::edit::EditCommand;
use iara_core::Profile;

pub const BUS_NAME: &str = "dev.iara.Mixer";
pub const OBJECT_PATH: &str = "/dev/iara/Mixer";
pub const INTERFACE: &str = "dev.iara.Mixer1";

/// Quem é avisado, com a nova versão, a cada mudança visível do estado.
pub type Notifier = Box<dyn Fn(u64) + Send + Sync>;

/// Estado de um aplicativo em relação ao canal desejado, como a interface o mostra (spec 8.11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AppState {
    Applied,
    Applying,
    /// Parte dos fluxos aplicada, parte não.
    Partial,
    /// Pedimos a mudança e o fluxo não foi para o destino.
    NotApplied,
    /// O fluxo está noutro canal do Iara (movido por fora): respeitado, sem briga.
    Elsewhere,
    /// O fluxo declara que não aceita ser movido.
    DontMove,
    /// Regra salva, mas o aplicativo não está tocando (não é recusa).
    Waiting,
    /// Sem identificação utilizável.
    Unmanaged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AppSource {
    /// Escolha temporária desta sessão.
    Session,
    /// Regra salva no perfil.
    Rule,
    /// Nenhuma regra: Não atribuídos.
    Default,
}

/// Um aplicativo para a interface: identidade, canal efetivo (`None` = Não atribuídos), de onde veio essa escolha e o estado real.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppEntry {
    pub key: Option<String>,
    pub display: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    pub source: AppSource,
    pub state: AppState,
    pub streams: u32,
}

impl AppEntry {
    pub fn identity(&self) -> iara_core::apps::AppIdentity {
        iara_core::apps::AppIdentity {
            app_id: self.app_id.clone(),
            binary: self.binary.clone(),
            name: self.name.clone(),
        }
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) struct AppsFile {
    #[serde(default, rename = "app")]
    pub apps: Vec<AppEntry>,
}

pub(crate) fn apps_to_toml(apps: &[AppEntry]) -> Result<String, String> {
    toml::to_string(&AppsFile {
        apps: apps.to_vec(),
    })
    .map_err(|e| e.to_string())
}

pub(crate) fn apps_from_toml(text: &str) -> Result<Vec<AppEntry>, String> {
    toml::from_str::<AppsFile>(text)
        .map(|f| f.apps)
        .map_err(|e| e.to_string())
}

/// Escolha temporária (só nesta sessão) para um aplicativo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionChoice {
    Channel(String),
    Unassigned,
    /// Reaplica a regra do perfil (apaga a escolha temporária).
    Clear,
}

/// Retrato do estado, como a interface o vê.
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    pub serial: u64,
    pub profile: Profile,
    pub connected: bool,
    pub persist_error: Option<String>,
    pub absent_devices: Vec<String>,
    pub reconnect_attempts: u32,
    pub apps: Vec<AppEntry>,
}

/// Quem atende o IPC: o serviço. Chamado de threads do D-Bus; deve responder com prazo.
pub trait Controller: Send + Sync + 'static {
    fn state(&self) -> Result<State, String>;
    /// Aplica a edição; devolve a nova versão ou o motivo da recusa.
    fn edit(&self, cmd: EditCommand) -> Result<u64, String>;
    /// Escolha temporária de canal para um aplicativo (chave de identidade); não altera o perfil.
    fn session_choice(&self, key: String, choice: SessionChoice) -> Result<u64, String>;
}
