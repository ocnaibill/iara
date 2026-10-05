use iara_core::topology::{diff, BranchSpec, NodeSpec, Plan, NODE_PREFIX};
use pipewire as pw;
use pw::spa::param::ParamType;
use pw::spa::pod::{serialize::PodSerializer, Object, Pod, Property, Value, ValueArray};
use pw::spa::sys::{SPA_PROP_channelVolumes, SPA_PROP_mute, SPA_TYPE_OBJECT_Props};
use pw::types::ObjectType;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::CString;
use std::io::Cursor;
use std::rc::Rc;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const OBSERVE_TIMEOUT: Duration = Duration::from_secs(5);
const TICK: Duration = Duration::from_millis(100);
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
}

/// Resultado de uma aplicação. `missing` lista os nós esperados que não apareceram no registro dentro do prazo;
/// “aplicado” só vale quando vazio. Ganho/mute são enviados ao nó, mas não relidos (a leitura é medida nas provas).
#[derive(Debug, PartialEq, Eq)]
pub struct ApplyReport {
    pub observed: usize,
    pub missing: Vec<String>,
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

pub struct Engine {
    tx: pw::channel::Sender<Command>,
    events: mpsc::Receiver<Event>,
    thread: Option<JoinHandle<()>>,
}

impl Engine {
    /// Conecta ao PipeWire da sessão; falha se o daemon não estiver acessível.
    pub fn start() -> Result<Self, EngineError> {
        let (tx, rx) = pw::channel::channel::<Command>();
        let (ev_tx, events) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("iara-audio".into())
            .spawn(move || run(rx, ev_tx, ready_tx))
            .map_err(|e| EngineError::Connect(e.to_string()))?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                tx,
                events,
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

    pub fn events(&self) -> &mpsc::Receiver<Event> {
        &self.events
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

struct ModuleHandle(*mut pw::sys::pw_impl_module);

impl Drop for ModuleHandle {
    fn drop(&mut self) {
        // SAFETY: ponteiro devolvido por pw_context_load_module e destruído uma única vez, na thread do loop.
        unsafe { pw::sys::pw_impl_module_destroy(self.0) };
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
        "node.description" => format!("Iara {}", spec.name.trim_start_matches(NODE_PREFIX)),
        "media.class" => "Audio/Sink",
        "audio.position" => "[FL FR]",
        "priority.session" => "0",
        "priority.driver" => "0",
        "state.restore-props" => "false",
        "state.restore-target" => "false",
        "iara.managed" => "true"
    }
}

fn loopback_args(b: &BranchSpec) -> String {
    format!(
        "{{ audio.position=[FL FR] node.name=\"{name}\" \
         capture.props={{ node.name=\"{inn}\" target.object=\"{from}\" stream.capture.sink=true node.passive=true node.dont-fallback=true {c} }} \
         playback.props={{ node.name=\"{out}\" target.object=\"{to}\" node.dont-fallback=true {c} }} }}",
        name = b.name,
        inn = in_name(&b.name),
        out = out_name(&b.name),
        from = b.from,
        to = b.to,
        c = COMMON_PROPS,
    )
}

fn check_pending(state: &Rc<RefCell<State>>, force: bool) {
    let mut st = state.borrow_mut();
    let Some(p) = st.pending.as_ref() else { return };
    let missing: Vec<String> = p
        .expected
        .iter()
        .filter(|n| !st.seen.contains_key(*n))
        .cloned()
        .collect();
    if missing.is_empty() || force || Instant::now() >= p.deadline {
        let p = st.pending.take().expect("pending presente");
        let observed = p.expected.len() - missing.len();
        let _ = p.reply.send(Ok(ApplyReport { observed, missing }));
    }
}

fn run(
    rx: pw::channel::Receiver<Command>,
    ev_tx: mpsc::Sender<Event>,
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
                    let _ = ev.send(Event::Disconnected);
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
                st_rm.borrow_mut().seen.retain(|_, (gid, _)| *gid != id);
            })
            .register()
    };

    let timer = {
        let st = state.clone();
        mainloop
            .loop_()
            .add_timer(move |_| check_pending(&st, false))
    };
    let _ = timer.update_timer(Some(TICK), Some(TICK));

    let _rx = {
        let ml = mainloop.clone();
        let st = state.clone();
        let core = core.clone();
        let ctx = context.clone();
        rx.attach(mainloop.loop_(), move |cmd| match cmd {
            Command::Shutdown => ml.quit(),
            Command::Apply(plan, reply) => apply(&st, &core, &ctx, *plan, reply),
        })
    };

    mainloop.run();

    // Fim da sessão: responde a quem espera e libera os objetos (módulos primeiro, depois os nós).
    {
        let mut st = state.borrow_mut();
        if let Some(p) = st.pending.take() {
            let _ = p.reply.send(Err(EngineError::Disconnected));
        }
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

    let mut st = state.borrow_mut();
    // 1) remove ramos antes dos nós que eles ligam
    for name in &rm_branches {
        st.modules.remove(name); // Drop destrói o módulo
        st.levels.remove(&out_name(name));
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
        let args = CString::new(loopback_args(b)).expect("sem NUL");
        // SAFETY: contexto vivo nesta thread; args é string C válida durante a chamada.
        let m = unsafe {
            pw::sys::pw_context_load_module(
                context.as_raw_ptr(),
                c"libpipewire-module-loopback".as_ptr(),
                args.as_ptr(),
                std::ptr::null_mut(),
            )
        };
        if m.is_null() {
            eprintln!("iara-audio: falha ao carregar o ramo {}", b.name);
        } else {
            st.modules.insert(b.name.clone(), ModuleHandle(m));
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
    st.applied = Some(plan);
    st.pending = Some(Pending {
        expected,
        deadline: Instant::now() + OBSERVE_TIMEOUT,
        reply,
    });
    drop(st);
    check_pending(state, false);
}
