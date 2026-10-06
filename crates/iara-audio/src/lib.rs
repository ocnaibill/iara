//! Backend PipeWire do Iara: hospeda no próprio processo os nós e ramos de um `Plan` e os mantém por diferença.
//!
//! Decisões da spec (6.3.1.x, provas 01–06): nós criados com `create_object("adapter")` sem `object.linger`;
//! ramos como `libpipewire-module-loopback` carregados no contexto do serviço; ganho e mute por `Props` no nó de saída
//! do ramo; todo objeto declara o opt-out da restauração de estado do WirePlumber. Esta conexão é uma sessão: se o
//! PipeWire cair, o motor emite `Event::Disconnected` e encerra; quem reconstrói é o serviço (spec 8.8).

mod defaults;
pub mod devices;
mod engine;
mod meters;
pub mod routing;

pub use devices::DeviceInfo;
pub use engine::{ApplyReport, Engine, EngineError, Event, EventSink};
pub use routing::{AppReport, AppRouteState, RouteTarget, StreamReport, StreamState};
