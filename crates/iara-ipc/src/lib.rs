//! Contrato D-Bus do Iara (barramento de sessão) entre o serviço e as interfaces.
//!
//! - Nome `dev.iara.Mixer`, objeto `/dev/iara/Mixer`, interface `dev.iara.Mixer1`.
//! - A interface nunca é dona do estado: lê um retrato (`GetState`) e muda coisas por comandos granulares; cada comando
//!   devolve a nova versão. O sinal `Changed(serial)` avisa que o retrato mudou (perfil ou status); a interface relê.
//! - Sem samples de áudio no IPC (spec 6.4). Medidores têm canal próprio: `SetMeters(bool)` é um pedido por cliente (some se o
//!   cliente cair) e, enquanto houver algum, o sinal `Levels(a(sd))` traz o pico linear de cada slider (~20 por segundo).
//! - Ganho em dB: `-inf` é o silêncio exato; fora disso vale a faixa −60..0.
//! - Este crate não depende do serviço: o servidor fala com um `Controller` abstrato; o cliente só precisa de zbus.

mod client;
mod server;

pub use client::{Client, ClientError, LevelsSubscription, Subscription};
pub use server::{serve, Server};

use serde::{Deserialize, Serialize};

use iara_core::edit::EditCommand;
use iara_core::Profile;

pub const BUS_NAME: &str = "dev.iara.Mixer";
pub const OBJECT_PATH: &str = "/dev/iara/Mixer";
pub const INTERFACE: &str = "dev.iara.Mixer1";

/// Quem recebe os níveis por slider: (chave em texto, pico linear).
pub type LevelsNotifier = Box<dyn Fn(Vec<(String, f64)>) + Send + Sync>;

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
    /// Sem regra e usando uma saída própria (escolhida no aplicativo): fica fora do mixer até ser associado a um canal.
    Outside,
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

/// Situação da saída principal do Iara como saída padrão do sistema (spec 8.2).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DefaultOutput {
    /// Desligado por configuração (desenvolvimento).
    Disabled,
    /// Ainda não instalada: falta uma saída física preferida presente (instalar sem ela deixaria o sistema mudo).
    #[default]
    Waiting,
    /// O Iara é a saída padrão: aplicativos novos entram pelo mixer.
    Active,
    /// O usuário escolheu outra saída depois: respeitado; aplicativos novos não passam pelo mixer.
    Released,
}

impl DefaultOutput {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Waiting => "waiting",
            Self::Active => "active",
            Self::Released => "released",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "disabled" => Self::Disabled,
            "waiting" => Self::Waiting,
            "active" => Self::Active,
            "released" => Self::Released,
            _ => return None,
        })
    }
}

/// Escolha temporária (só nesta sessão) para um aplicativo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionChoice {
    Channel(String),
    Unassigned,
    /// Reaplica a regra do perfil (apaga a escolha temporária).
    Clear,
}

/// O retrato como trafega no D-Bus (assinatura `tsbsasussa(ssb)`): versão, perfil em TOML, conectado, erro de gravação,
/// dispositivos ausentes, tentativas de reconexão, aplicativos em TOML, saída padrão, perfis (id, nome, legível) e dispositivos
/// físicos de áudio (chave, descrição, é saída).
pub(crate) type StateWire = (
    u64,
    String,
    bool,
    String,
    Vec<String>,
    u32,
    String,
    String,
    Vec<(String, String, bool)>,
    Vec<(String, String, bool)>,
);

/// Dispositivo físico de áudio que o usuário pode escolher (fone/alto-falantes ou microfone).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceEntry {
    /// Chave persistente (hoje o `node.name`).
    pub key: String,
    pub description: String,
    /// `true` = saída; `false` = entrada.
    pub output: bool,
}

/// Perfil na lista do retrato.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileEntry {
    pub id: String,
    pub name: String,
    /// `false` se o arquivo do perfil não pôde ser lido (aparece para o usuário poder tratá-lo).
    pub readable: bool,
}

/// Operações sobre perfis (spec 8.4, 8.10). `Create`/`Duplicate` devolvem o id novo; as demais, o id do perfil ativo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileOp {
    /// Troca o perfil ativo (salva o atual, valida o destino, prepara o novo e só então remove o obsoleto).
    Switch(String),
    /// Perfil novo com os canais iniciais; herda os dispositivos do ativo. Não troca para ele.
    Create(String),
    /// Cópia independente de um perfil.
    Duplicate {
        id: String,
        name: String,
    },
    Rename {
        id: String,
        name: String,
    },
    /// Vai para a lixeira (recuperável); o perfil ativo e o último perfil não podem ser excluídos.
    Delete(String),
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
    pub default_output: DefaultOutput,
    pub profiles: Vec<ProfileEntry>,
    /// Dispositivos físicos presentes agora.
    pub devices: Vec<DeviceEntry>,
}

/// Quem atende o IPC: o serviço. Chamado de threads do D-Bus; deve responder com prazo.
pub trait Controller: Send + Sync + 'static {
    fn state(&self) -> Result<State, String>;
    /// Aplica a edição; devolve a nova versão ou o motivo da recusa.
    fn edit(&self, cmd: EditCommand) -> Result<u64, String>;
    /// Escolha temporária de canal para um aplicativo (chave de identidade); não altera o perfil.
    fn session_choice(&self, key: String, choice: SessionChoice) -> Result<u64, String>;
    /// “Desligar mixer / voltar ao áudio normal”: restaura a saída padrão anterior (se ainda for a do Iara) e encerra o serviço.
    fn deactivate(&self) -> Result<(), String>;
    /// Operação sobre perfis; devolve o id novo (criar/duplicar) ou o do perfil ativo.
    fn profile_op(&self, op: ProfileOp) -> Result<String, String>;
    /// Algum cliente quer (ou nenhum quer mais) medidores ao vivo. Chamado só quando o conjunto de pedidos muda de vazio
    /// para não vazio e vice-versa.
    fn meters(&self, enabled: bool) -> Result<(), String>;
}
