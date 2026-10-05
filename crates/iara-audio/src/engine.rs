use iara_core::topology::{
    diff, BranchSpec, DeviceDirection, DeviceLink, NodeSpec, Plan, SourceSpec, NODE_PREFIX,
};
use pipewire as pw;
use pw::spa::param::ParamType;
use pw::spa::pod::{serialize::PodSerializer, Object, Pod, Property, Value, ValueArray};
use pw::spa::sys::{SPA_PROP_channelVolumes, SPA_PROP_mute, SPA_TYPE_OBJECT_Props};
use pw::types::ObjectType;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{c_void, CString};
use std::io::Cursor;
use std::rc::Rc;
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const OBSERVE_TIMEOUT: Duration = Duration::from_secs(5);
const TICK: Duration = Duration::from_millis(100);
/// Espera mínima entre tentativas de religar um dispositivo (evita laço se o módulo se descarregar de novo).
const DEVICE_RETRY: Duration = Duration::from_secs(2);
/// Opt-out da restauração de volume/mute/destino do WirePlumber (prova 03) e marca de propriedade do Iara.
const COMMON_PROPS: &str = "state.restore-props=false state.restore-target=false iara.managed=true";

#[derive(Debug, PartialEq, Eq)]
pub enum EngineError {
    Connect(String),
    Disconnected,
    Superseded,
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connect(m) => write!(f, "não foi possível conectar ao PipeWire: {m}"),
            Self::Disconnected => write!(f, "conexão com o PipeWire perdida"),
            Self::Superseded => write!(f, "aplicação substituída por outra mais recente"),
        }
    }
}

impl std::error::Error for EngineError {}

#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    Disconnected,
    /// O dispositivo físico preferido (chave persistente) não está no sistema; a ligação foi removida, sem fallback.
    DeviceAbsent(String),
    /// O dispositivo preferido voltou; a ligação foi recriada.
    DeviceBack(String),
}

/// Resultado de uma aplicação. `missing` lista os nós esperados que não apareceram no registro dentro do prazo;
/// “aplicado” só vale quando vazio. Ganho/mute são enviados ao nó, mas não relidos (a leitura é medida nas provas).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyReport {
    pub observed: usize,
    pub missing: Vec<String>,
    /// Dispositivos físicos desejados que não estão presentes (não contam como `missing`).
    pub absent_devices: Vec<String>,
}

impl ApplyReport {
    pub fn is_complete(&self) -> bool {
        self.missing.is_empty()
    }
}

type Reply = mpsc::Sender<Result<ApplyReport, EngineError>>;

enum Command {
    Apply(Box<Plan>, Reply),
    Shutdown,
}

/// Destino dos eventos do motor. Chamado na thread do motor: deve ser rápido e não bloquear (ex.: enviar a um canal).
pub type EventSink = Arc<dyn Fn(Event) + Send + Sync>;

pub struct Engine {
    tx: pw::channel::Sender<Command>,
    /// Só existe quando o motor foi iniciado com `start()`; com `start_with` os eventos vão ao sink do chamador.
    events: Option<mpsc::Receiver<Event>>,
    thread: Option<JoinHandle<()>>,
}

impl Engine {
    /// Conecta ao PipeWire da sessão; falha se o daemon não estiver acessível.
    pub fn start() -> Result<Self, EngineError> {
        let (ev_tx, events) = mpsc::channel();
        let ev_tx = std::sync::Mutex::new(ev_tx);
        let sink: EventSink = Arc::new(move |e| {
            if let Ok(tx) = ev_tx.lock() {
                let _ = tx.send(e);
            }
        });
        let mut engine = Self::start_with(sink)?;
        engine.events = Some(events);
        Ok(engine)
    }

    /// Como `start`, mas entrega os eventos ao `sink` (ex.: o canal único do serviço, para esperar sem polling).
    pub fn start_with(sink: EventSink) -> Result<Self, EngineError> {
        let (tx, rx) = pw::channel::channel::<Command>();
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("iara-audio".into())
            .spawn(move || run(rx, sink, ready_tx))
            .map_err(|e| EngineError::Connect(e.to_string()))?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                tx,
                events: None,
                thread: Some(thread),
            }),
            Ok(Err(m)) => Err(EngineError::Connect(m)),
            Err(_) => Err(EngineError::Connect(
                "thread encerrou antes de conectar".into(),
            )),
        }
    }

    /// Aplica o plano por diferença e espera os nós aparecerem no registro (até 5 s).
    pub fn apply(&self, plan: Plan) -> Result<ApplyReport, EngineError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx
            .send(Command::Apply(Box::new(plan), reply_tx))
            .map_err(|_| EngineError::Disconnected)?;
        reply_rx.recv().map_err(|_| EngineError::Disconnected)?
    }

    /// Eventos acumulados (só para motores criados com `start()`).
    pub fn events(&self) -> impl Iterator<Item = Event> + '_ {
        self.events.iter().flat_map(|r| r.try_iter())
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Módulo carregado no nosso contexto. O módulo loopback se descarrega sozinho quando o alvo some (prova 05b), então
/// o ponteiro pode ficar inválido sem aviso: um listener do evento `destroy` mantém `alive` e evita destruí-lo duas vezes.
struct ModuleHandle {
    module: *mut pw::sys::pw_impl_module,
    alive: *const Cell<bool>,
    _hook: Box<pw::spa::sys::spa_hook>,
    _events: Box<pw::sys::pw_impl_module_events>,
}

unsafe extern "C" fn on_module_destroy(data: *mut c_void) {
    // SAFETY: `data` é o ponteiro de `alive`, válido enquanto o ModuleHandle existir.
    unsafe { (*(data as *const Cell<bool>)).set(false) };
}

impl ModuleHandle {
    fn new(module: *mut pw::sys::pw_impl_module) -> Self {
        let alive = Rc::into_raw(Rc::new(Cell::new(true)));
        // SAFETY: spa_hook é POD inicializável com zeros; os dois Box mantêm endereço estável até o Drop.
        let mut hook: Box<pw::spa::sys::spa_hook> = Box::new(unsafe { std::mem::zeroed() });
        let events = Box::new(pw::sys::pw_impl_module_events {
            version: 0,
            destroy: Some(on_module_destroy),
            free: None,
            initialized: None,
            registered: None,
        });
        // SAFETY: módulo recém-carregado e vivo; hook/events/alive sobrevivem ao módulo (liberados só no Drop).
        unsafe {
            pw::sys::pw_impl_module_add_listener(module, &mut *hook, &*events, alive as *mut c_void)
        };
        Self {
            module,
            alive,
            _hook: hook,
            _events: events,
        }
    }

    fn is_alive(&self) -> bool {
        // SAFETY: `alive` vem de Rc::into_raw e só é liberado no Drop.
        unsafe { (*self.alive).get() }
    }
}

impl Drop for ModuleHandle {
    fn drop(&mut self) {
        if self.is_alive() {
            // SAFETY: módulo ainda vivo, destruído uma única vez, na thread do loop; o evento destroy zera `alive`.
            unsafe { pw::sys::pw_impl_module_destroy(self.module) };
        }
        // SAFETY: devolve a contagem criada em `new`.
        unsafe { drop(Rc::from_raw(self.alive)) };
    }
}

struct Pending {
    expected: Vec<String>,
    deadline: Instant,
    reply: Reply,
}

#[derive(Default)]
struct State {
    applied: Option<Plan>,
    owned_nodes: HashMap<String, pw::node::Node>,
    modules: HashMap<String, ModuleHandle>,
    /// Nós `iara.*` vistos no registro, por nome: (id global, proxy ligado).
    seen: HashMap<String, (u32, pw::node::Node)>,
    /// Nível desejado por nó de saída de ramo: (volume linear, mute). Reaplicado sempre que o nó aparece.
    levels: HashMap<String, (f32, bool)>,
    pending: Option<Pending>,
    /// Ligações desejadas com dispositivos físicos e os módulos que as realizam (nome da ligação → (físico, módulo)).
    devices: Vec<DeviceLink>,
    device_modules: HashMap<String, (String, ModuleHandle)>,
    device_present: HashMap<String, bool>,
    retry_after: HashMap<String, Instant>,
    /// Todos os nós do registro (id → node.name); base para saber se um dispositivo físico está presente.
    nodes_present: HashMap<u32, String>,
}

fn props_pod(prop: u32, value: Value) -> Vec<u8> {
    let obj = Object {
        type_: SPA_TYPE_OBJECT_Props,
        id: ParamType::Props.as_raw(),
        properties: vec![Property::new(prop, value)],
    };
    PodSerializer::serialize(Cursor::new(Vec::new()), &Value::Object(obj))
        .expect("serialização de pod em memória")
        .0
        .into_inner()
}

fn set_level(node: &pw::node::Node, volume: f32, muted: bool) {
    let vol = props_pod(
        SPA_PROP_channelVolumes,
        Value::ValueArray(ValueArray::Float(vec![volume, volume])),
    );
    let mute = props_pod(SPA_PROP_mute, Value::Bool(muted));
    for bytes in [vol, mute] {
        node.set_param(
            ParamType::Props,
            0,
            Pod::from_bytes(&bytes).expect("pod válido"),
        );
    }
}

fn out_name(branch: &str) -> String {
    format!("{branch}.out")
}

fn in_name(branch: &str) -> String {
    format!("{branch}.in")
}

fn node_props(spec: &NodeSpec) -> pw::properties::PropertiesBox {
    pw::properties::properties! {
        "factory.name" => "support.null-audio-sink",
        "node.name" => spec.name.as_str(),
        "node.description" => spec.description.as_str(),
        "media.class" => "Audio/Sink",
        "audio.position" => "[FL FR]",
        "priority.session" => "0",
        "priority.driver" => "0",
        "state.restore-props" => "false",
        "state.restore-target" => "false",
        "iara.managed" => "true"
    }
}

/// Valor entre aspas em SPA-JSON: escapa `\` e `"` (os ids e chaves já são validados no core; defesa em profundidade).
fn q(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Loopback entre dois nós. `from_is_sink`: lê o monitor de um sink (barramentos do Iara, saída) em vez de uma fonte.
/// Sempre `node.dont-fallback`: o WirePlumber nunca troca o alvo por outro dispositivo (spec 8.5/8.6).
fn link_args(name: &str, from: &str, from_is_sink: bool, to: &str) -> String {
    let (name, from, to) = (q(name), q(from), q(to));
    format!(
        "{{ audio.position=[FL FR] node.name=\"{name}\" \
         capture.props={{ node.name=\"{inn}\" target.object=\"{from}\" stream.capture.sink={from_is_sink} node.passive=true node.dont-fallback=true {c} }} \
         playback.props={{ node.name=\"{out}\" target.object=\"{to}\" node.dont-fallback=true {c} }} }}",
        inn = in_name(&name),
        out = out_name(&name),
        c = COMMON_PROPS,
    )
}

fn loopback_args(b: &BranchSpec) -> String {
    link_args(&b.name, &b.from, true, &b.to)
}

fn device_args(d: &DeviceLink) -> String {
    match d.direction {
        DeviceDirection::Output => link_args(&d.name, &d.bus, true, &d.physical),
        DeviceDirection::Input => link_args(&d.name, &d.physical, false, &d.bus),
    }
}

/// Fonte virtual selecionável por outros aplicativos: o lado de captura lê o monitor de `from`; o lado de reprodução
/// aparece como `Audio/Source` (não `Audio/Source/Virtual`: PipeWire 1.6.9 falha com ela, ver docs/provas/registro.md). Prioridade de sessão 0 para nunca virar fonte padrão por conta própria.
fn source_args(src: &SourceSpec) -> String {
    format!(
        "{{ audio.position=[FL FR] \
         capture.props={{ node.name=\"{cap}\" target.object=\"{from}\" stream.capture.sink=true node.passive=true node.dont-fallback=true {c} }} \
         playback.props={{ node.name=\"{name}\" node.description=\"{desc}\" media.class=Audio/Source priority.session=0 {c} }} }}",
        cap = source_capture_name(&src.name),
        from = src.from,
        name = src.name,
        desc = src.description,
        c = COMMON_PROPS,
    )
}

fn source_capture_name(source: &str) -> String {
    format!("{source}.cap")
}

fn load_module(context: &pw::context::ContextRc, args: &str) -> Option<ModuleHandle> {
    let args = CString::new(args).expect("sem NUL");
    // SAFETY: contexto vivo nesta thread; args é string C válida durante a chamada.
    let m = unsafe {
        pw::sys::pw_context_load_module(
            context.as_raw_ptr(),
            c"libpipewire-module-loopback".as_ptr(),
            args.as_ptr(),
            std::ptr::null_mut(),
        )
    };
    (!m.is_null()).then(|| ModuleHandle::new(m))
}

fn device_present(st: &State, d: &DeviceLink) -> bool {
    st.nodes_present.values().any(|n| *n == d.physical)
}

/// Cria/remove as ligações com dispositivos físicos conforme a presença no registro. Nunca recria com outro
/// dispositivo: o plano é a única fonte do dispositivo preferido (a escolha do usuário durante a ausência é um novo plano).
fn reconcile_devices(st: &mut State, context: &pw::context::ContextRc, ev: &EventSink) {
    let desired = st.devices.clone();
    let stale: Vec<String> = st
        .device_modules
        .iter()
        .filter(|(name, (physical, handle))| {
            desired.iter().find(|d| &d.name == *name).is_none_or(|d| {
                d.physical != *physical || !handle.is_alive() || !device_present(st, d)
            })
        })
        .map(|(name, _)| name.clone())
        .collect();
    for name in stale {
        st.device_modules.remove(&name);
    }
    for d in &desired {
        let present = device_present(st, d);
        match (st.device_present.insert(d.name.clone(), present), present) {
            (None | Some(true), false) => {
                ev(Event::DeviceAbsent(d.physical.clone()));
            }
            (Some(false), true) => {
                ev(Event::DeviceBack(d.physical.clone()));
            }
            _ => {}
        }
        let ready = st
            .retry_after
            .get(&d.name)
            .is_none_or(|t| Instant::now() >= *t);
        if present && ready && !st.device_modules.contains_key(&d.name) {
            st.retry_after
                .insert(d.name.clone(), Instant::now() + DEVICE_RETRY);
            match load_module(context, &device_args(d)) {
                Some(m) => {
                    st.device_modules
                        .insert(d.name.clone(), (d.physical.clone(), m));
                }
                None => eprintln!("iara-audio: falha ao ligar o dispositivo {}", d.physical),
            }
        }
    }
    st.device_present
        .retain(|name, _| desired.iter().any(|d| &d.name == name));
}

fn check_pending(state: &Rc<RefCell<State>>, force: bool) {
    let mut st = state.borrow_mut();
    let Some(p) = st.pending.as_ref() else { return };
    let mut expected = p.expected.clone();
    for name in st.device_modules.keys() {
        expected.push(in_name(name));
        expected.push(out_name(name));
    }
    let missing: Vec<String> = expected
        .iter()
        .filter(|n| !st.seen.contains_key(*n))
        .cloned()
        .collect();
    if missing.is_empty() || force || Instant::now() >= p.deadline {
        let absent_devices = st
            .devices
            .iter()
            .filter(|d| !device_present(&st, d))
            .map(|d| d.physical.clone())
            .collect();
        let p = st.pending.take().expect("pending presente");
        let observed = expected.len() - missing.len();
        let _ = p.reply.send(Ok(ApplyReport {
            observed,
            missing,
            absent_devices,
        }));
    }
}

fn run(
    rx: pw::channel::Receiver<Command>,
    ev_tx: EventSink,
    ready: mpsc::Sender<Result<(), String>>,
) {
    pw::init();
    let setup = (|| {
        let mainloop = pw::main_loop::MainLoopRc::new(None).map_err(|e| e.to_string())?;
        let context = pw::context::ContextRc::new(&mainloop, None).map_err(|e| e.to_string())?;
        let core = context.connect_rc(None).map_err(|e| e.to_string())?;
        let registry = core.get_registry_rc().map_err(|e| e.to_string())?;
        Ok::<_, String>((mainloop, context, core, registry))
    })();
    let (mainloop, context, core, registry) = match setup {
        Ok(v) => v,
        Err(m) => {
            let _ = ready.send(Err(m));
            return;
        }
    };
    let _ = ready.send(Ok(()));

    let state: Rc<RefCell<State>> = Rc::default();
    let disconnected = Rc::new(std::cell::Cell::new(false));

    let _core_listener = {
        let ml = mainloop.clone();
        let dc = disconnected.clone();
        let ev = ev_tx.clone();
        core.add_listener_local()
            .error(move |id, _seq, res, _msg| {
                // -EPIPE no objeto core: o daemon fechou a conexão (reinício ou queda do PipeWire).
                if id == pw::core::PW_ID_CORE && res == -32 {
                    dc.set(true);
                    ev(Event::Disconnected);
                    ml.quit();
                }
            })
            .register()
    };

    let _reg_listener = {
        let st_add = state.clone();
        let st_rm = state.clone();
        let reg = registry.clone();
        registry
            .add_listener_local()
            .global(move |g| {
                if g.type_ != ObjectType::Node {
                    return;
                }
                let Some(name) = g.props.and_then(|p| p.get("node.name")) else {
                    return;
                };
                // Presença de qualquer nó (inclusive dispositivos físicos) alimenta o reconciliador de dispositivos.
                st_add
                    .borrow_mut()
                    .nodes_present
                    .insert(g.id, name.to_owned());
                if !name.starts_with(NODE_PREFIX) {
                    return;
                }
                if let Ok(node) = reg.bind::<pw::node::Node, _>(g) {
                    // Estado desejado é do Iara: reaplica ganho/mute assim que o nó aparece (spec 6.3.1.1).
                    if let Some((v, m)) = st_add.borrow().levels.get(name) {
                        set_level(&node, *v, *m);
                    }
                    st_add
                        .borrow_mut()
                        .seen
                        .insert(name.to_owned(), (g.id, node));
                    check_pending(&st_add, false);
                }
            })
            .global_remove(move |id| {
                let mut st = st_rm.borrow_mut();
                st.seen.retain(|_, (gid, _)| *gid != id);
                st.nodes_present.remove(&id);
            })
            .register()
    };

    let timer = {
        let st = state.clone();
        let ctx = context.clone();
        let ev = ev_tx.clone();
        mainloop.loop_().add_timer(move |_| {
            reconcile_devices(&mut st.borrow_mut(), &ctx, &ev);
            check_pending(&st, false);
        })
    };
    let _ = timer.update_timer(Some(TICK), Some(TICK));

    let _rx = {
        let ml = mainloop.clone();
        let st = state.clone();
        let core = core.clone();
        let ctx = context.clone();
        let ev = ev_tx.clone();
        rx.attach(mainloop.loop_(), move |cmd| match cmd {
            Command::Shutdown => ml.quit(),
            Command::Apply(plan, reply) => apply(&st, &core, &ctx, &ev, *plan, reply),
        })
    };

    mainloop.run();

    // Fim da sessão: responde a quem espera e libera os objetos (módulos primeiro, depois os nós).
    {
        let mut st = state.borrow_mut();
        if let Some(p) = st.pending.take() {
            let _ = p.reply.send(Err(EngineError::Disconnected));
        }
        st.device_modules.clear();
        st.modules.clear();
        st.owned_nodes.clear();
        st.seen.clear();
    }
    drop(timer);
    let _ = disconnected;
}

fn apply(
    state: &Rc<RefCell<State>>,
    core: &pw::core::CoreRc,
    context: &pw::context::ContextRc,
    ev: &EventSink,
    plan: Plan,
    reply: Reply,
) {
    {
        let mut st = state.borrow_mut();
        if let Some(old) = st.pending.take() {
            let _ = old.reply.send(Err(EngineError::Superseded));
        }
    }
    let empty = Plan {
        nodes: vec![],
        branches: vec![],
        sources: vec![],
        devices: vec![],
    };
    let current = state.borrow().applied.clone().unwrap_or(empty);
    let d = diff(&current, &plan);
    let (rm_branches, rm_nodes): (Vec<String>, Vec<String>) = (
        d.remove_branches.iter().map(|b| b.name.clone()).collect(),
        d.remove_nodes.iter().map(|n| n.name.clone()).collect(),
    );
    let add_nodes: Vec<NodeSpec> = d.add_nodes.iter().map(|n| (*n).clone()).collect();
    let add_branches: Vec<BranchSpec> = d.add_branches.iter().map(|b| (*b).clone()).collect();
    let retune: Vec<BranchSpec> = d.retune.iter().map(|b| (*b).clone()).collect();
    let rm_sources: Vec<String> = d.remove_sources.iter().map(|x| x.name.clone()).collect();
    let add_sources: Vec<SourceSpec> = d.add_sources.iter().map(|x| (*x).clone()).collect();

    let mut st = state.borrow_mut();
    // 1) remove ramos antes dos nós que eles ligam
    for name in &rm_branches {
        st.modules.remove(name); // Drop destrói o módulo
        st.levels.remove(&out_name(name));
    }
    for name in &rm_sources {
        st.modules.remove(name);
    }
    for name in &rm_nodes {
        if let Some(node) = st.owned_nodes.remove(name) {
            let _ = core.destroy_object(node);
        }
    }
    // 2) cria nós e ramos
    for spec in &add_nodes {
        match core.create_object::<pw::node::Node>("adapter", &node_props(spec)) {
            Ok(n) => {
                st.owned_nodes.insert(spec.name.clone(), n);
            }
            Err(e) => eprintln!("iara-audio: nó {}: {e}", spec.name),
        }
    }
    for b in &add_branches {
        st.levels
            .insert(out_name(&b.name), (b.volume as f32, b.muted));
        match load_module(context, &loopback_args(b)) {
            Some(m) => {
                st.modules.insert(b.name.clone(), m);
            }
            None => eprintln!("iara-audio: falha ao carregar o ramo {}", b.name),
        }
    }
    for src in &add_sources {
        match load_module(context, &source_args(src)) {
            Some(m) => {
                st.modules.insert(src.name.clone(), m);
            }
            None => eprintln!("iara-audio: falha ao carregar a fonte {}", src.name),
        }
    }
    // 3) só ganho/mute
    for b in &retune {
        let key = out_name(&b.name);
        st.levels.insert(key.clone(), (b.volume as f32, b.muted));
        if let Some((_, node)) = st.seen.get(&key) {
            set_level(node, b.volume as f32, b.muted);
        }
    }
    let mut expected: Vec<String> = plan.nodes.iter().map(|n| n.name.clone()).collect();
    for b in &plan.branches {
        expected.push(in_name(&b.name));
        expected.push(out_name(&b.name));
    }
    for src in &plan.sources {
        expected.push(source_capture_name(&src.name));
        expected.push(src.name.clone());
    }
    st.devices = plan.devices.clone();
    reconcile_devices(&mut st, context, ev);
    st.applied = Some(plan);
    st.pending = Some(Pending {
        expected,
        deadline: Instant::now() + OBSERVE_TIMEOUT,
        reply,
    });
    drop(st);
    check_pending(state, false);
}
