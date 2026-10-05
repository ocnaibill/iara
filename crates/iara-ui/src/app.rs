//! Janela GTK4 do mixer. Só apresenta o retrato do serviço e envia comandos; não guarda configuração própria.
//! Os valores vêm de `model` (testado sem GTK); aqui ficam widgets, estilo e a ligação com o backend.

use crate::backend::{Backend, OnUpdate, Update};
use crate::model::{
    chatmix_view, columns, position_to_gain, slug, status_lines, ColumnKind, ColumnView, SendView,
    StatusKind,
};
use gtk::prelude::*;
use gtk::{gdk, glib};
use iara_core::edit::{EditCommand, MicSend, SendKind};
use iara_core::Gain;
use iara_ipc::State;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

/// Id do aplicativo GTK. Não pode ser o nome do serviço (`dev.iara.Mixer`): o GApplication registra o id no
/// barramento de sessão e colidiria com o dono do nome.
const APP_ID: &str = "dev.iara.Panel";

const CSS: &str = "
window.iara { background: #0e1b22; color: #d8e6ea; }
.iara headerbar { background: #0b151b; color: #d8e6ea; box-shadow: none; border-bottom: 1px solid #1c2f38; }
.iara .profile { color: #7e98a1; }
.iara .column { background: #14262e; border-radius: 10px; padding: 10px 8px 12px 8px; }
.iara .column.master { background: #17303a; }
.iara .col-title { font-weight: 800; letter-spacing: 1px; font-size: 13px; }
.iara .strip-caption { color: #7e98a1; font-size: 9.5px; letter-spacing: 0.4px; }
.iara .value { font-feature-settings: 'tnum'; font-size: 12px; }
.iara .note { color: #f2b35e; font-size: 10px; }
.iara scale trough { background: #1f3a45; min-width: 6px; min-height: 6px; border-radius: 6px; }
.iara scale highlight { background: #5ad1c5; border-radius: 6px; border: none; }
.iara scale.tx highlight { background: #f2b35e; }
.iara scale slider { background: #e8f2f4; min-width: 20px; min-height: 12px; margin: 0; border-radius: 4px; border: none; box-shadow: 0 1px 3px rgba(0,0,0,.5); }
.iara .off scale highlight { background: #3a5560; }
.iara .off .value { color: #7e98a1; }
.iara .head { min-height: 34px; }
.iara button.flat-btn { background: #1b323c; border: 1px solid #25444f; border-radius: 8px; color: #d8e6ea; }
.iara button.toggle { background: #1b323c; border: 1px solid #25444f; color: #d8e6ea; border-radius: 8px; min-width: 32px; min-height: 28px; padding: 0 6px; }
.iara button.toggle:checked.enable { background: #1d5a54; border-color: #5ad1c5; }
.iara button.toggle:checked.enable.tx { background: #5a431d; border-color: #f2b35e; }
.iara button.toggle:checked.mute { background: #5b2a2a; border-color: #ef6f6c; color: #ffd9d8; }
.iara .banner { border-radius: 8px; padding: 6px 10px; color: #f1f6f7; }
.iara .banner.st-error { background: #4a2326; border: 1px solid #ef6f6c; }
.iara .banner.st-warning { background: #4a3a1c; border: 1px solid #f2b35e; }
.iara .banner.st-info { background: #1d3a46; border: 1px solid #5ad1c5; }
.iara .add { background: transparent; border: 1px dashed #2c4a56; border-radius: 10px; color: #7e98a1; min-width: 56px; }
.iara .chatmix { background: #14262e; border-radius: 10px; padding: 8px 14px; }
.iara .muted-text { color: #7e98a1; }
";

#[derive(Clone)]
pub struct Options {
    pub demo: bool,
    pub screenshot: Option<PathBuf>,
    pub bus_name: String,
}

type Emit = Rc<dyn Fn(EditCommand)>;

struct Strip {
    root: gtk::Box,
    enable: gtk::ToggleButton,
    scale: gtk::Scale,
    value: gtk::Label,
    note: gtk::Label,
    mute: gtk::ToggleButton,
    dragging: Rc<Cell<bool>>,
}

impl Strip {
    fn apply(&self, v: &SendView) {
        self.enable.set_active(v.enabled);
        self.mute.set_active(v.muted);
        if !self.dragging.get() {
            self.scale.set_value(v.position);
        }
        self.value.set_text(&v.label);
        self.note.set_text(v.note.as_deref().unwrap_or(""));
        if v.enabled {
            self.root.remove_css_class("off");
        } else {
            self.root.add_css_class("off");
        }
    }
}

struct Handlers {
    gain: Box<dyn Fn(Gain)>,
    mute: Box<dyn Fn(bool)>,
    enable: Option<Box<dyn Fn(bool)>>,
}

fn label(text: &str, class: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class(class);
    l
}

fn toggle(icon: &str, tooltip: &str, classes: &[&str]) -> gtk::ToggleButton {
    let b = gtk::ToggleButton::builder()
        .icon_name(icon)
        .tooltip_text(tooltip)
        .build();
    b.add_css_class("toggle");
    for c in classes {
        b.add_css_class(c);
    }
    b.update_property(&[gtk::accessible::Property::Label(tooltip)]);
    b
}

fn build_strip(
    caption: &str,
    icon: &str,
    transmission: bool,
    updating: &Rc<Cell<bool>>,
    h: Handlers,
) -> Strip {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
    root.set_halign(gtk::Align::Center);
    let tx = ["enable", if transmission { "tx" } else { "rx" }];
    let enable = toggle(icon, &format!("{caption}: participa do mix"), &tx);
    if h.enable.is_none() {
        // sem habilitação (MASTER): o botão fica invisível mas ocupa o lugar, para alinhar os sliders entre colunas
        enable.set_opacity(0.0);
        enable.set_sensitive(false);
        enable.set_focusable(false);
    }
    root.append(&label(caption, "strip-caption"));
    root.append(&enable);

    let scale = gtk::Scale::with_range(gtk::Orientation::Vertical, 0.0, 1.0, 0.001);
    scale.set_inverted(true);
    scale.set_draw_value(false);
    scale.set_vexpand(true);
    scale.set_size_request(30, 200);
    if transmission {
        scale.add_css_class("tx");
    }
    scale.update_property(&[gtk::accessible::Property::Label(&format!(
        "Volume — {caption}"
    ))]);
    let dragging = Rc::new(Cell::new(false));
    let press = gtk::GestureClick::new();
    press.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let d = dragging.clone();
        press.connect_pressed(move |_, _, _, _| d.set(true));
        let d = dragging.clone();
        press.connect_released(move |_, _, _, _| d.set(false));
        let d = dragging.clone();
        press.connect_stopped(move |_| d.set(false));
    }
    scale.add_controller(press);
    root.append(&scale);

    let value = label("0,0 dB", "value");
    let note = label("", "note");
    note.set_height_request(14);
    let mute = toggle(
        "audio-volume-high-symbolic",
        &format!("Silenciar — {caption}"),
        &["mute"],
    );
    mute.connect_toggled(|b| {
        // o estado não depende só da cor: o ícone muda
        b.set_icon_name(if b.is_active() {
            "audio-volume-muted-symbolic"
        } else {
            "audio-volume-high-symbolic"
        });
    });
    root.append(&value);
    root.append(&note);
    root.append(&mute);

    {
        let (u, f) = (updating.clone(), h.gain);
        scale.connect_value_changed(move |s| {
            if !u.get() {
                f(position_to_gain(s.value()));
            }
        });
    }
    {
        let (u, f) = (updating.clone(), h.mute);
        mute.connect_toggled(move |b| {
            if !u.get() {
                f(b.is_active());
            }
        });
    }
    if let Some(f) = h.enable {
        let u = updating.clone();
        enable.connect_toggled(move |b| {
            if !u.get() {
                f(b.is_active());
            }
        });
    }
    Strip {
        root,
        enable,
        scale,
        value,
        note,
        mute,
        dragging,
    }
}

struct MicWidgets {
    global_mute: gtk::ToggleButton,
    input: gtk::Scale,
    input_value: gtk::Label,
    apps: Strip,
}

struct Column {
    kind: ColumnKind,
    id: String,
    title: String,
    root: gtk::Box,
    personal: Strip,
    transmission: Strip,
    mic: Option<MicWidgets>,
}

impl Column {
    fn apply(&self, v: &ColumnView) {
        self.personal.apply(&v.personal);
        self.transmission.apply(&v.transmission);
        if let (Some(w), Some(m)) = (&self.mic, &v.mic) {
            w.global_mute.set_active(m.global_mute);
            w.input.set_value(m.input_position);
            w.input_value.set_text(&m.input_label);
            w.apps.apply(&m.applications);
        }
    }
}

fn kind_handlers(kind: &ColumnKind, id: &str, send: SendKind, emit: &Emit) -> Handlers {
    let e = emit.clone();
    let id = id.to_owned();
    match kind {
        ColumnKind::Master => Handlers {
            gain: Box::new({
                let e = e.clone();
                move |gain| e(EditCommand::SetMasterGain { send, gain })
            }),
            mute: Box::new(move |muted| e(EditCommand::SetMasterMute { send, muted })),
            enable: None,
        },
        ColumnKind::Channel => Handlers {
            gain: Box::new({
                let (e, id) = (e.clone(), id.clone());
                move |gain| {
                    e(EditCommand::SetChannelGain {
                        channel: id.clone(),
                        send,
                        gain,
                    })
                }
            }),
            mute: Box::new({
                let (e, id) = (e.clone(), id.clone());
                move |muted| {
                    e(EditCommand::SetChannelMute {
                        channel: id.clone(),
                        send,
                        muted,
                    })
                }
            }),
            enable: Some(Box::new(move |enabled| {
                e(EditCommand::SetChannelEnabled {
                    channel: id.clone(),
                    send,
                    enabled,
                })
            })),
        },
        ColumnKind::Mic => {
            let m = if send == SendKind::Personal {
                MicSend::Personal
            } else {
                MicSend::Transmission
            };
            Handlers {
                gain: Box::new({
                    let e = e.clone();
                    move |gain| e(EditCommand::SetMicSendGain { send: m, gain })
                }),
                mute: Box::new({
                    let e = e.clone();
                    move |muted| e(EditCommand::SetMicSendMute { send: m, muted })
                }),
                enable: Some(Box::new(move |enabled| {
                    e(EditCommand::SetMicSendEnabled { send: m, enabled })
                })),
            }
        }
    }
}

fn manage_menu(
    id: &str,
    name: &str,
    existing: &Rc<RefCell<Vec<String>>>,
    emit: &Emit,
) -> gtk::MenuButton {
    let _ = existing;
    let pop = gtk::Popover::new();
    let bx = gtk::Box::new(gtk::Orientation::Vertical, 6);
    bx.set_margin_top(8);
    bx.set_margin_bottom(8);
    bx.set_margin_start(8);
    bx.set_margin_end(8);
    let entry = gtk::Entry::builder().text(name).max_length(64).build();
    let rename = gtk::Button::with_label("Renomear");
    let remove = gtk::Button::with_label("Remover canal");
    remove.add_css_class("destructive-action");
    bx.append(&entry);
    bx.append(&rename);
    bx.append(&remove);
    pop.set_child(Some(&bx));
    {
        let (e, id, entry, pop) = (emit.clone(), id.to_owned(), entry.clone(), pop.clone());
        rename.connect_clicked(move |_| {
            let name = entry.text().trim().to_owned();
            if !name.is_empty() {
                e(EditCommand::RenameChannel {
                    channel: id.clone(),
                    name,
                });
                pop.popdown();
            }
        });
    }
    {
        // remoção em duas etapas: o primeiro clique pede confirmação
        let (e, id, pop_c) = (emit.clone(), id.to_owned(), pop.clone());
        remove.connect_clicked(move |b| {
            if b.label().as_deref() == Some("Confirmar remoção") {
                e(EditCommand::RemoveChannel {
                    channel: id.clone(),
                    destination: None,
                });
                pop_c.popdown();
            } else {
                b.set_label("Confirmar remoção");
            }
        });
        let remove2 = remove.clone();
        pop.connect_closed(move |_| remove2.set_label("Remover canal"));
    }
    let mb = gtk::MenuButton::builder()
        .icon_name("emblem-system-symbolic")
        .tooltip_text("Configurações do canal")
        .popover(&pop)
        .build();
    mb.add_css_class("flat");
    mb.update_property(&[gtk::accessible::Property::Label("Configurações do canal")]);
    mb
}

fn build_column(
    v: &ColumnView,
    emit: &Emit,
    updating: &Rc<Cell<bool>>,
    existing: &Rc<RefCell<Vec<String>>>,
) -> Column {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    root.add_css_class("column");
    if v.kind == ColumnKind::Master {
        root.add_css_class("master");
    }
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    head.add_css_class("head");
    let title = label(&v.title, "col-title");
    title.set_hexpand(true);
    title.set_xalign(0.0);
    head.append(&title);
    if v.kind == ColumnKind::Channel {
        head.append(&manage_menu(&v.id, &v.title, existing, emit));
    }
    root.append(&head);

    let mut mic = None;
    if let Some(m) = &v.mic {
        let gm = toggle(
            "microphone-disabled-symbolic",
            "Silenciar o microfone em todos os destinos",
            &["mute"],
        );
        gm.set_label("Mute global");
        gm.set_active(m.global_mute);
        {
            let (e, u) = (emit.clone(), updating.clone());
            gm.connect_toggled(move |b| {
                if !u.get() {
                    e(EditCommand::SetMicGlobalMute(b.is_active()));
                }
            });
        }
        root.append(&gm);
        let input = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.001);
        input.set_draw_value(false);
        input.update_property(&[gtk::accessible::Property::Label(
            "Ganho de entrada do microfone",
        )]);
        let input_value = label(&m.input_label, "value");
        {
            let (e, u) = (emit.clone(), updating.clone());
            input.connect_value_changed(move |s| {
                if !u.get() {
                    e(EditCommand::SetMicInputGain(position_to_gain(s.value())));
                }
            });
        }
        let e = emit.clone();
        let apps_handlers = Handlers {
            gain: Box::new({
                let e = e.clone();
                move |gain| {
                    e(EditCommand::SetMicSendGain {
                        send: MicSend::Applications,
                        gain,
                    })
                }
            }),
            mute: Box::new({
                let e = e.clone();
                move |muted| {
                    e(EditCommand::SetMicSendMute {
                        send: MicSend::Applications,
                        muted,
                    })
                }
            }),
            enable: Some(Box::new(move |enabled| {
                e(EditCommand::SetMicSendEnabled {
                    send: MicSend::Applications,
                    enabled,
                })
            })),
        };
        let apps = build_strip(
            "APLICATIVOS",
            "audio-input-microphone-symbolic",
            true,
            updating,
            apps_handlers,
        );
        mic = Some(MicWidgets {
            global_mute: gm,
            input,
            input_value,
            apps,
        });
    }

    let strips = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    strips.set_vexpand(true);
    strips.set_halign(gtk::Align::Center);
    let personal = build_strip(
        "ESCUTA",
        "audio-headphones-symbolic",
        false,
        updating,
        kind_handlers(&v.kind, &v.id, SendKind::Personal, emit),
    );
    let transmission = build_strip(
        "TRANSMISSÃO",
        "network-wireless-symbolic",
        true,
        updating,
        kind_handlers(&v.kind, &v.id, SendKind::Transmission, emit),
    );
    strips.append(&personal.root);
    strips.append(&transmission.root);
    root.append(&strips);

    if let Some(w) = &mic {
        let ex = gtk::Expander::new(Some("Microfone para aplicativos"));
        let inner = gtk::Box::new(gtk::Orientation::Vertical, 6);
        inner.append(&label("Ganho de entrada", "strip-caption"));
        inner.append(&w.input);
        inner.append(&w.input_value);
        inner.append(&w.apps.root);
        ex.set_child(Some(&inner));
        root.append(&ex);
    }
    root.set_size_request(
        if v.kind == ColumnKind::Master {
            176
        } else {
            160
        },
        -1,
    );
    let col = Column {
        kind: v.kind.clone(),
        id: v.id.clone(),
        title: v.title.clone(),
        root,
        personal,
        transmission,
        mic,
    };
    col.apply(v);
    col
}

struct ChatMixBar {
    root: gtk::Box,
    first: gtk::Label,
    second: gtk::Label,
    scale: gtk::Scale,
}

struct Ui {
    window: gtk::ApplicationWindow,
    banner: gtk::Box,
    profile: gtk::Label,
    row: gtk::Box,
    add_button: gtk::MenuButton,
    chatmix: ChatMixBar,
    cols: RefCell<Vec<Column>>,
    updating: Rc<Cell<bool>>,
    existing: Rc<RefCell<Vec<String>>>,
    emit: Emit,
    transient: RefCell<Option<String>>,
}

impl Ui {
    fn render_banner(&self, state: Option<&State>, unavailable: Option<&str>) {
        while let Some(c) = self.banner.first_child() {
            self.banner.remove(&c);
        }
        let mut lines = state.map(status_lines).unwrap_or_default();
        if let Some(m) = unavailable {
            lines.insert(
                0,
                crate::model::StatusLine {
                    kind: StatusKind::Error,
                    text: crate::model::unavailable_text(m),
                },
            );
        }
        if let Some(t) = self.transient.borrow().clone() {
            lines.push(crate::model::StatusLine {
                kind: StatusKind::Warning,
                text: t,
            });
        }
        for l in &lines {
            let (class, prefix) = match l.kind {
                StatusKind::Error => ("st-error", "✖ "),
                StatusKind::Warning => ("st-warning", "⚠ "),
                StatusKind::Info => ("st-info", "ℹ "),
            };
            let lab = gtk::Label::new(Some(&format!("{prefix}{}", l.text)));
            lab.set_xalign(0.0);
            lab.set_wrap(true);
            lab.add_css_class("banner");
            lab.add_css_class(class);
            self.banner.append(&lab);
        }
        self.banner.set_visible(!lines.is_empty());
    }

    fn apply_state(&self, state: &State, unavailable: Option<&str>) {
        self.updating.set(true);
        *self.existing.borrow_mut() = state
            .profile
            .channels
            .iter()
            .map(|c| c.id.clone())
            .collect();
        self.profile.set_text(&format!("— {}", state.profile.name));
        let views = columns(&state.profile);
        let same = {
            let cols = self.cols.borrow();
            cols.len() == views.len()
                && cols
                    .iter()
                    .zip(&views)
                    .all(|(c, v)| c.kind == v.kind && c.id == v.id && c.title == v.title)
        };
        if !same {
            while let Some(c) = self.row.first_child() {
                self.row.remove(&c);
            }
            let built: Vec<Column> = views
                .iter()
                .map(|v| build_column(v, &self.emit, &self.updating, &self.existing))
                .collect();
            for c in &built {
                self.row.append(&c.root);
            }
            self.row.append(&self.add_button);
            *self.cols.borrow_mut() = built;
        } else {
            for (c, v) in self.cols.borrow().iter().zip(&views) {
                c.apply(v);
            }
        }
        match chatmix_view(&state.profile) {
            Some(cm) => {
                self.chatmix.first.set_text(&cm.first);
                self.chatmix.second.set_text(&cm.second);
                if !self.chatmix.scale.has_focus() {
                    self.chatmix.scale.set_value(cm.position);
                }
                self.chatmix.root.set_visible(true);
            }
            None => self.chatmix.root.set_visible(false),
        }
        self.render_banner(Some(state), unavailable);
        self.row
            .set_sensitive(state.connected || unavailable.is_none());
        self.updating.set(false);
    }
}

thread_local! {
    static UI: RefCell<Option<Rc<Ui>>> = const { RefCell::new(None) };
}

fn on_main(f: impl FnOnce(&Rc<Ui>) + Send + 'static) {
    glib::MainContext::default().invoke(move || {
        UI.with(|u| {
            if let Some(ui) = u.borrow().as_ref() {
                f(ui);
            }
        })
    });
}

/// Retrato de demonstração para `--demo`: mostra a interface sem serviço nem PipeWire.
pub fn demo_state() -> State {
    use iara_core::edit::{apply, EditCommand as E};
    let mut p = iara_core::initial_profile();
    let g = |db: f64| Gain::from_db(db).unwrap();
    for cmd in [
        E::SetChannelGain {
            channel: "game".into(),
            send: SendKind::Personal,
            gain: g(-9.0),
        },
        E::SetChannelGain {
            channel: "chat".into(),
            send: SendKind::Personal,
            gain: g(-3.0),
        },
        E::SetChannelGain {
            channel: "chat".into(),
            send: SendKind::Transmission,
            gain: g(-12.0),
        },
        E::SetChannelGain {
            channel: "media".into(),
            send: SendKind::Personal,
            gain: g(-24.0),
        },
        E::SetChannelMute {
            channel: "media".into(),
            send: SendKind::Transmission,
            muted: true,
        },
        E::SetMasterGain {
            send: SendKind::Personal,
            gain: g(-6.0),
        },
        E::SetMicInputGain(g(-18.0)),
        E::SetChatMixPosition(0.35),
    ] {
        p = apply(&p, &cmd).expect("demo").0;
    }
    State {
        serial: 1,
        profile: p,
        connected: true,
        persist_error: None,
        absent_devices: vec!["alsa_output.usb-fone-exemplo".into()],
        reconnect_attempts: 0,
    }
}

fn save_png(window: &gtk::ApplicationWindow, path: &std::path::Path) -> Result<(), String> {
    let paintable = gtk::WidgetPaintable::new(Some(window));
    let (w, h) = (window.width() as f64, window.height() as f64);
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(snapshot.upcast_ref::<gdk::Snapshot>(), w, h);
    let node = snapshot.to_node().ok_or("nada renderizado")?;
    let renderer = window
        .native()
        .and_then(|n| n.renderer())
        .ok_or("sem renderer")?;
    let texture = renderer.render_texture(&node, None);
    texture.save_to_png(path).map_err(|e| e.to_string())
}

fn build_ui(app: &gtk::Application, opts: &Options) {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(CSS);
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_application_prefer_dark_theme(true);
    }

    let backend: Rc<RefCell<Option<Backend>>> = Rc::default();
    let emit: Emit = {
        let b = backend.clone();
        Rc::new(move |cmd| {
            if let Some(b) = b.borrow().as_ref() {
                b.send(cmd);
            }
        })
    };

    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Iara")
        .default_width(1320)
        .default_height(700)
        .build();
    window.add_css_class("iara");
    let header = gtk::HeaderBar::new();
    let title = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    title.append(&label("Iara", "col-title"));
    let profile = label("", "profile");
    title.append(&profile);
    header.set_title_widget(Some(&title));
    window.set_titlebar(Some(&header));

    let outer = gtk::Box::new(gtk::Orientation::Vertical, 10);
    for set in [
        gtk::Widget::set_margin_top,
        gtk::Widget::set_margin_bottom,
        gtk::Widget::set_margin_start,
        gtk::Widget::set_margin_end,
    ] {
        set(outer.upcast_ref(), 12);
    }
    let banner = gtk::Box::new(gtk::Orientation::Vertical, 6);
    banner.set_visible(false);
    outer.append(&banner);

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.set_vexpand(true);
    let scroller = gtk::ScrolledWindow::builder()
        .child(&row)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();
    outer.append(&scroller);

    // ChatMix
    let cm_root = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    cm_root.add_css_class("chatmix");
    cm_root.set_visible(false);
    let first = label("", "strip-caption");
    let second = label("", "strip-caption");
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, -1.0, 1.0, 0.01);
    scale.set_hexpand(true);
    scale.set_draw_value(false);
    scale.add_mark(0.0, gtk::PositionType::Bottom, None);
    scale.update_property(&[gtk::accessible::Property::Label(
        "ChatMix: equilíbrio entre os dois canais na escuta",
    )]);
    let center = gtk::Button::with_label("Centro");
    center.add_css_class("flat-btn");
    cm_root.append(&label("CHATMIX", "col-title"));
    cm_root.append(&first);
    cm_root.append(&scale);
    cm_root.append(&second);
    cm_root.append(&center);
    outer.append(&cm_root);
    window.set_child(Some(&outer));

    let updating = Rc::new(Cell::new(false));
    {
        let (e, u) = (emit.clone(), updating.clone());
        scale.connect_value_changed(move |s| {
            if !u.get() {
                e(EditCommand::SetChatMixPosition(
                    (s.value() * 100.0).round() / 100.0,
                ));
            }
        });
        let s = scale.clone();
        center.connect_clicked(move |_| s.set_value(0.0));
    }

    // "+ Canal"
    let existing: Rc<RefCell<Vec<String>>> = Rc::default();
    let add_pop = gtk::Popover::new();
    let add_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
    for set in [
        gtk::Widget::set_margin_top,
        gtk::Widget::set_margin_bottom,
        gtk::Widget::set_margin_start,
        gtk::Widget::set_margin_end,
    ] {
        set(add_box.upcast_ref(), 8);
    }
    let name_entry = gtk::Entry::builder()
        .placeholder_text("Nome do canal")
        .max_length(64)
        .build();
    let add_ok = gtk::Button::with_label("Criar canal");
    add_box.append(&name_entry);
    add_box.append(&add_ok);
    add_pop.set_child(Some(&add_box));
    {
        let (e, ex, entry, pop) = (
            emit.clone(),
            existing.clone(),
            name_entry.clone(),
            add_pop.clone(),
        );
        add_ok.connect_clicked(move |_| {
            let name = entry.text().trim().to_owned();
            if let Some(id) = slug(&name, &ex.borrow()) {
                e(EditCommand::AddChannel { id, name });
                entry.set_text("");
                pop.popdown();
            }
        });
    }
    let add_button = gtk::MenuButton::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text("Novo canal")
        .popover(&add_pop)
        .valign(gtk::Align::Start)
        .build();
    add_button.add_css_class("add");
    add_button.update_property(&[gtk::accessible::Property::Label("Novo canal")]);

    let ui = Rc::new(Ui {
        window: window.clone(),
        banner,
        profile,
        row,
        add_button,
        chatmix: ChatMixBar {
            root: cm_root,
            first,
            second,
            scale,
        },
        cols: RefCell::new(Vec::new()),
        updating,
        existing,
        emit,
        transient: RefCell::new(None),
    });
    UI.with(|u| *u.borrow_mut() = Some(ui.clone()));

    if opts.demo {
        ui.apply_state(&demo_state(), None);
    } else {
        // enquanto o primeiro retrato não chega, mostra que está procurando o serviço
        ui.render_banner(None, Some("procurando…"));
        let cb: OnUpdate = std::sync::Arc::new(|u: Update| {
            on_main(move |ui| match u {
                Update::State(st) => {
                    *ui.transient.borrow_mut() = None;
                    ui.apply_state(&st, None);
                }
                Update::Unavailable(m) => {
                    ui.row.set_sensitive(false);
                    ui.render_banner(None, Some(&m));
                }
                Update::Rejected(m) => {
                    *ui.transient.borrow_mut() = Some(format!("Comando recusado: {m}"));
                    ui.render_banner(None, None);
                    let weak = glib::SendWeakRef::from(ui.window.downgrade());
                    let _ = weak;
                    glib::timeout_add_seconds_local_once(6, || {
                        UI.with(|u| {
                            if let Some(ui) = u.borrow().as_ref() {
                                *ui.transient.borrow_mut() = None;
                            }
                        });
                    });
                }
            });
        });
        *backend.borrow_mut() = Some(Backend::spawn(opts.bus_name.clone(), cb));
    }

    window.present();
    if let Some(path) = opts.screenshot.clone() {
        let (w, app) = (window.clone(), app.clone());
        let delay = std::env::var("IARA_UI_SHOT_DELAY_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(900);
        glib::timeout_add_local_once(Duration::from_millis(delay), move || {
            match save_png(&w, &path) {
                Ok(()) => eprintln!("captura salva em {}", path.display()),
                Err(e) => eprintln!("falha na captura: {e}"),
            }
            app.quit();
        });
    }
}

pub fn run(opts: Options) -> glib::ExitCode {
    let app = gtk::Application::builder().application_id(APP_ID).build();
    app.connect_activate(move |app| build_ui(app, &opts));
    // os argumentos próprios (--demo etc.) já foram lidos; não repassá-los ao GTK
    app.run_with_args::<&str>(&[])
}
