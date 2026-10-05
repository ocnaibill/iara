//! Sonda descartável (spec 6.3.1): um processo Rust hospeda nós PipeWire e os controla via Props.
//! Comandos por stdin: `null NOME` | `loop NOME ORIGEM DESTINO` | `vol NÓ LINEAR` | `mute NÓ true|false`
//! | `list` | `quit`. Todos os nós usam o prefixo iara_probe_ e somem quando o processo termina.
use pipewire as pw;
use pw::spa::param::ParamType;
use pw::spa::pod::{serialize::PodSerializer, Object, Pod, Property, Value, ValueArray};
use pw::spa::sys::{SPA_PROP_channelVolumes, SPA_PROP_mute, SPA_TYPE_OBJECT_Props};
use pw::types::ObjectType;
use std::{cell::RefCell, collections::HashMap, ffi::CString, io::BufRead, io::Cursor, rc::Rc};

enum Cmd {
    Line(String),
}

fn props_pod(prop: u32, value: Value) -> Vec<u8> {
    let obj = Object {
        type_: SPA_TYPE_OBJECT_Props,
        id: ParamType::Props.as_raw(),
        properties: vec![Property::new(prop, value)],
    };
    PodSerializer::serialize(Cursor::new(Vec::new()), &Value::Object(obj))
        .expect("serialize")
        .0
        .into_inner()
}

fn main() {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None).expect("mainloop");
    let context = pw::context::ContextRc::new(&mainloop, None).expect("context");
    let core = context.connect_rc(None).expect("connect");
    let registry = Rc::new(core.get_registry_rc().expect("registry"));

    // Nós vistos no registro (todos os iara_probe_*), por node.name.
    let nodes: Rc<RefCell<HashMap<String, pw::node::Node>>> = Rc::default();
    let _own: Rc<RefCell<Vec<pw::node::Node>>> = Rc::default(); // proxies que mantêm vivos os nós criados por nós
    let _reg_listener = {
        let nodes = nodes.clone();
        let reg = registry.clone();
        registry
            .add_listener_local()
            .global(move |g| {
                if g.type_ != ObjectType::Node {
                    return;
                }
                let Some(name) = g.props.and_then(|p| p.get("node.name")) else { return };
                if name.starts_with("iara_probe_") {
                    if let Ok(node) = reg.bind::<pw::node::Node, _>(g) {
                        nodes.borrow_mut().insert(name.to_owned(), node);
                    }
                }
            })
            .register()
    };

    let (tx, rx) = pw::channel::channel::<Cmd>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            if tx.send(Cmd::Line(line)).is_err() {
                break;
            }
        }
        let _ = tx.send(Cmd::Line("quit".into()));
    });

    let ml = mainloop.clone();
    let ctx = context.clone();
    let core2 = core.clone();
    let own = _own.clone();
    let _rx = rx.attach(mainloop.loop_(), move |Cmd::Line(line)| {
        let w: Vec<&str> = line.split_whitespace().collect();
        match w.as_slice() {
            ["null", name] => {
                let p = pw::properties::properties! {
                    "factory.name" => "support.null-audio-sink",
                    "node.name" => format!("iara_probe_{name}"),
                    "media.class" => "Audio/Sink",
                    "audio.position" => "[FL FR]",
                    "priority.session" => "0",
                    "priority.driver" => "0",
                    "state.restore-props" => "false",
                    "state.restore-target" => "false"
                };
                match core2.create_object::<pw::node::Node>("adapter", &p) {
                    Ok(n) => own.borrow_mut().push(n),
                    Err(e) => eprintln!("null {name}: {e}"),
                }
            }
            ["loop", name, src, dst] => {
                let args = format!(
                    "{{ audio.position=[FL FR] node.name=iara_probe_{name} \
                     capture.props={{ node.name=iara_probe_{name}_in target.object=iara_probe_{src} stream.capture.sink=true node.passive=true node.dont-fallback=true state.restore-props=false state.restore-target=false }} \
                     playback.props={{ node.name=iara_probe_{name}_out target.object=iara_probe_{dst} node.dont-fallback=true state.restore-props=false state.restore-target=false }} }}"
                );
                let a = CString::new(args).unwrap();
                let m = unsafe {
                    pw::sys::pw_context_load_module(
                        ctx.as_raw_ptr(),
                        c"libpipewire-module-loopback".as_ptr(),
                        a.as_ptr(),
                        std::ptr::null_mut(),
                    )
                };
                if m.is_null() {
                    eprintln!("loop {name}: falha ao carregar o módulo");
                }
            }
            ["vol", node, lin] => {
                let v: f32 = lin.parse().unwrap_or(1.0);
                let bytes = props_pod(
                    SPA_PROP_channelVolumes,
                    Value::ValueArray(ValueArray::Float(vec![v, v])),
                );
                set(&nodes.borrow(), node, &bytes);
            }
            ["mute", node, b] => {
                let bytes = props_pod(SPA_PROP_mute, Value::Bool(*b == "true"));
                set(&nodes.borrow(), node, &bytes);
            }
            ["list"] => {
                let mut k: Vec<_> = nodes.borrow().keys().cloned().collect();
                k.sort();
                println!("nós: {k:?}");
            }
            ["quit"] => ml.quit(),
            [] => {}
            _ => eprintln!("comando desconhecido: {line}"),
        }
    });

    mainloop.run();
}

fn set(nodes: &HashMap<String, pw::node::Node>, name: &str, bytes: &[u8]) {
    match nodes.get(&format!("iara_probe_{name}")) {
        Some(n) => n.set_param(ParamType::Props, 0, Pod::from_bytes(bytes).expect("pod")),
        None => eprintln!("nó {name} ainda não visto no registro"),
    }
}
