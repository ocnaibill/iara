//! Janela GTK4 do mixer. Só apresenta o retrato do serviço e envia comandos; não guarda configuração própria.
//! Os valores vêm de `model` (testado sem GTK); aqui ficam widgets, estilo e a ligação com o backend.

use crate::backend::{Backend, OnUpdate, Update};
use crate::meters::{MeterBoard, MeterView};
use crate::model::{
    active_profile_label, chatmix_view, chips_by_column, columns, device_choices, move_app,
    position_to_gain, profile_rows, reapply_rule, slug, status_lines, AppChip, ColumnKind,
    ColumnView, SendView, StatusKind, UiCommand,
};
use gtk::prelude::*;
use gtk::{gdk, glib};
use iara_core::edit::{EditCommand, MicSend, SendKind};
use iara_core::meter::MeterKey;
use iara_core::Gain;
use iara_ipc::{AppEntry, AppSource, AppState, DefaultOutput, ProfileOp, State};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Id do aplicativo GTK. Não pode ser o nome do serviço (`dev.iara.Mixer`): o GApplication registra o id no
/// barramento de sessão e colidiria com o dono do nome.
const APP_ID: &str = "dev.iara.Panel";

const CSS: &str = "
window.iara { background: #0e1b22; color: #d8e6ea; }
.iara headerbar { background: #0b151b; color: #d8e6ea; box-shadow: none; border-bottom: 1px solid #1c2f38; }
.iara menubutton.profile-button > button { background: transparent; border: none; color: #d8e6ea; font-weight: 600; }
.iara .column { background: #14262e; border-radius: 10px; padding: 10px 8px 12px 8px; }
.iara .column.master { background: #17303a; }
.iara .col-title { font-weight: 800; letter-spacing: 1px; font-size: 13px; }
.iara .strip-caption { color: #7e98a1; font-size: 9.5px; letter-spacing: 0.4px; }
.iara .value { font-feature-settings: 'tnum'; font-size: 12px; }
.iara .note { color: #f2b35e; font-size: 10px; }
.iara scale trough { background: transparent; min-width: 6px; min-height: 6px; border-radius: 6px; }
.iara scale.vstrip trough { min-width: 14px; }
.iara scale highlight { background: #5ad1c5; border-radius: 6px; border: none; margin: 0; min-width: 0; min-height: 0; }
.iara scale.vstrip highlight { background: rgba(90, 209, 197, 0.30); border: 1px solid #5ad1c5; }
.iara scale.tx highlight { background: #f2b35e; }
.iara scale.vstrip.tx highlight { background: rgba(242, 179, 94, 0.30); border: 1px solid #f2b35e; }
.iara scale.hstrip trough { background: #1f3a45; }
.iara scale slider { background: #e8f2f4; min-width: 20px; min-height: 12px; margin: 0; border-radius: 4px; border: none; box-shadow: 0 1px 3px rgba(0,0,0,.5); }
.iara .off scale highlight { background: #3a5560; }
.iara .off scale.vstrip highlight { background: rgba(126, 152, 161, 0.25); border: 1px solid #3a5560; }
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
.iara .apps-title { color: #7e98a1; font-size: 9.5px; letter-spacing: 0.4px; margin-top: 4px; }
.iara menubutton.chip > button { background: #1b323c; border: 1px solid #2c4a56; border-radius: 8px; color: #d8e6ea; padding: 2px 8px; min-height: 22px; font-size: 12px; }
.iara menubutton.chip > button arrow { min-width: 0; min-height: 0; -gtk-icon-size: 0px; margin: 0; }
.iara menubutton.chip.temp > button { border-style: dashed; border-color: #f2b35e; }
.iara menubutton.chip.idle > button { color: #7e98a1; background: transparent; }
.iara menubutton.chip.attention > button { border-color: #ef6f6c; }
.iara headerbar button:checked, .iara headerbar button.toggle:checked { background: #1d5a54; border: 1px solid #5ad1c5; color: #ffffff; }
.iara button.flat-btn.armed { background: #5b2a2a; border-color: #ef6f6c; color: #ffd9d8; }
.iara .column.drop-target { border: 1px dashed #5ad1c5; background: #1a3640; }
.iara .apps-empty { color: #4f6a74; font-size: 11px; }
";

#[derive(Clone)]
pub struct Options {
    pub demo: bool,
    pub screenshot: Option<PathBuf>,
    pub bus_name: String,
}

type Emit = Rc<dyn Fn(EditCommand)>;
type EmitUi = Rc<dyn Fn(UiCommand)>;

/// O que as etiquetas de aplicativos precisam saber: enviar comandos, a lista atual e os canais (para o menu "Mover para").
#[derive(Clone)]
struct AppsCtx {
    emit: EmitUi,
    apps: Rc<RefCell<Vec<AppEntry>>>,
    /// (id, nome) dos canais do perfil, na ordem das colunas.
    channels: Rc<RefCell<Vec<(String, String)>>>,
    /// Interruptor do cabeçalho: mover só nesta sessão em vez de salvar a regra.
    session_only: Rc<Cell<bool>>,
    /// Engrenagem da coluna MASTER: escolha do fone (saída) e do microfone que o Iara vai rotear.
    device_button: gtk::MenuButton,
}

/// Seletores de dispositivo físico dentro do popover da engrenagem do MASTER.
struct DevicePicker {
    output: gtk::DropDown,
    input: gtk::DropDown,
    output_hint: gtk::Label,
    input_hint: gtk::Label,
    /// Chaves das opções atuais de cada seletor (índice → chave; `None` = nenhum).
    output_keys: Rc<RefCell<Vec<Option<String>>>>,
    input_keys: Rc<RefCell<Vec<Option<String>>>>,
    sig: RefCell<String>,
}

fn dropdown_selector(
    caption: &str,
    accessible: &str,
    keys: &Rc<RefCell<Vec<Option<String>>>>,
    updating: &Rc<Cell<bool>>,
    on_pick: impl Fn(Option<String>) + 'static,
) -> (gtk::Box, gtk::DropDown, gtk::Label) {
    let bx = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let cap = label(caption, "strip-caption");
    bx.append(&cap);
    let dd = gtk::DropDown::from_strings(&["…"]);
    // sem isto o leitor de tela anuncia só o valor atual ("Nenhuma saída…"), não a finalidade do seletor
    dd.update_property(&[gtk::accessible::Property::Label(accessible)]);
    dd.update_relation(&[gtk::accessible::Relation::LabelledBy(&[cap.upcast_ref()])]);
    let hint = label("", "note");
    hint.set_xalign(0.0);
    hint.set_wrap(true);
    {
        let (keys, updating) = (keys.clone(), updating.clone());
        dd.connect_selected_notify(move |d| {
            if updating.get() {
                return;
            }
            if let Some(key) = keys.borrow().get(d.selected() as usize) {
                on_pick(key.clone());
            }
        });
    }
    bx.append(&dd);
    bx.append(&hint);
    (bx, dd, hint)
}

/// Engrenagem + popover "Dispositivos" da coluna MASTER (spec 8.5, 8.6, 8.12).
fn build_device_menu(emit: &Emit, updating: &Rc<Cell<bool>>) -> (gtk::MenuButton, DevicePicker) {
    let pop = gtk::Popover::new();
    let bx = gtk::Box::new(gtk::Orientation::Vertical, 10);
    for set in [
        gtk::Widget::set_margin_top,
        gtk::Widget::set_margin_bottom,
        gtk::Widget::set_margin_start,
        gtk::Widget::set_margin_end,
    ] {
        set(bx.upcast_ref(), 12);
    }
    bx.set_size_request(300, -1);
    bx.append(&label("DISPOSITIVOS", "col-title"));
    let intro = label(
        "O Iara cuida do roteamento: a escuta sai pela saída escolhida e o microfone entra pelo que você escolher. \
         Cada perfil guarda os seus.",
        "muted-text",
    );
    intro.set_wrap(true);
    intro.set_xalign(0.0);
    bx.append(&intro);
    let output_keys: Rc<RefCell<Vec<Option<String>>>> = Rc::default();
    let input_keys: Rc<RefCell<Vec<Option<String>>>> = Rc::default();
    let (e1, e2) = (emit.clone(), emit.clone());
    let (out_box, output, output_hint) = dropdown_selector(
        "SAÍDA (FONE OU ALTO-FALANTES)",
        "Saída do mix pessoal (fone ou alto-falantes)",
        &output_keys,
        updating,
        move |key| e1(EditCommand::SetPreferredOutput(key)),
    );
    let (in_box, input, input_hint) = dropdown_selector(
        "MICROFONE",
        "Entrada do microfone",
        &input_keys,
        updating,
        move |key| e2(EditCommand::SetPreferredMicrophone(key)),
    );
    bx.append(&out_box);
    bx.append(&in_box);
    pop.set_child(Some(&bx));
    let button = gtk::MenuButton::builder()
        .icon_name("emblem-system-symbolic")
        .tooltip_text("Dispositivos: fone e microfone")
        .popover(&pop)
        .build();
    button.add_css_class("flat");
    button.update_property(&[gtk::accessible::Property::Label(
        "Configurações de dispositivos: fone e microfone",
    )]);
    (
        button,
        DevicePicker {
            output,
            input,
            output_hint,
            input_hint,
            output_keys,
            input_keys,
            sig: RefCell::new(String::new()),
        },
    )
}

impl DevicePicker {
    /// Refaz as opções só quando mudam (reconstruir com a lista aberta a fecharia) e marca a escolha atual.
    fn refresh(&self, state: &State) {
        let (outs, out_sel) = device_choices(state, true);
        let (ins, in_sel) = device_choices(state, false);
        let sig = format!("{outs:?}{out_sel}{ins:?}{in_sel}");
        if *self.sig.borrow() == sig {
            return;
        }
        *self.sig.borrow_mut() = sig;
        for (dd, choices, sel, keys, hint) in [
            (
                &self.output,
                &outs,
                out_sel,
                &self.output_keys,
                &self.output_hint,
            ),
            (
                &self.input,
                &ins,
                in_sel,
                &self.input_keys,
                &self.input_hint,
            ),
        ] {
            *keys.borrow_mut() = choices.iter().map(|c| c.key.clone()).collect();
            let labels: Vec<&str> = choices.iter().map(|c| c.label.as_str()).collect();
            dd.set_model(Some(&gtk::StringList::new(&labels)));
            dd.set_selected(sel as u32);
            hint.set_text(if choices[sel].absent {
                "Este dispositivo não está conectado agora; a escolha continua registrada e volta sozinha quando ele reaparecer."
            } else if choices[sel].key.is_none() {
                "Sem dispositivo escolhido: este lado fica sem som."
            } else {
                ""
            });
        }
    }
}

struct Strip {
    root: gtk::Box,
    enable: gtk::ToggleButton,
    scale: gtk::Scale,
    value: gtk::Label,
    note: gtk::Label,
    mute: gtk::ToggleButton,
    dragging: Rc<Cell<bool>>,
    /// Qual medidor alimenta a barra deste slider e o que ela desenha agora.
    meter_key: MeterKey,
    meter_view: Rc<Cell<MeterView>>,
    meter_area: gtk::DrawingArea,
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

/// Passos de teclado de um slider de ganho: posição 1,0 = 60 dB, então 1 dB = 1/60.
fn set_gain_steps(scale: &gtk::Scale) {
    let adj = scale.adjustment();
    adj.set_step_increment(1.0 / 60.0);
    adj.set_page_increment(6.0 / 60.0);
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
    owner: &str,
    caption: &str,
    icon: &str,
    transmission: bool,
    meter_key: MeterKey,
    updating: &Rc<Cell<bool>>,
    h: Handlers,
) -> Strip {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
    root.set_halign(gtk::Align::Center);
    let tx = ["enable", if transmission { "tx" } else { "rx" }];
    // nomes acessíveis incluem a coluna: sem isso todos os sliders de "ESCUTA" soam iguais para um leitor de tela
    let enable = toggle(icon, &format!("{owner} — {caption}: participa do mix"), &tx);
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
    scale.add_css_class("vstrip");
    if transmission {
        scale.add_css_class("tx");
    }
    // "volume da escuta", "volume da transmissão", "volume para aplicativos"
    let what = if caption == "APLICATIVOS" {
        "para aplicativos".to_owned()
    } else {
        format!("da {}", caption.to_lowercase())
    };
    scale.update_property(&[gtk::accessible::Property::Label(&format!(
        "{owner} — volume {what}"
    ))]);
    // teclado: 1 dB por seta e 6 dB por Page (os passos padrão eram de 0,06 dB, inviáveis para ajuste fino)
    set_gain_steps(&scale);
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
    // medidor ao vivo: a barra é desenhada ATRÁS do slider (o fundo da calha é dela), então o puxador e o preenchimento do
    // ganho ficam por cima e continuam recebendo o mouse e o teclado
    let meter_view = Rc::new(Cell::new(MeterView::default()));
    let meter_area = build_meter_area(&scale, &meter_view);
    let stack = gtk::Overlay::new();
    stack.set_child(Some(&meter_area));
    stack.add_overlay(&scale);
    stack.set_measure_overlay(&scale, true);
    root.append(&stack);

    let value = label("0,0 dB", "value");
    let note = label("", "note");
    note.set_height_request(14);
    let mute = toggle(
        "audio-volume-high-symbolic",
        &format!("{owner} — silenciar {}", caption.to_lowercase()),
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
        meter_key,
        meter_view,
        meter_area,
    }
}

/// Onde a barra vai, em coordenadas da área de desenho: centro horizontal e extensão vertical da calha do slider, e o trecho
/// do eixo que o puxador percorre (a barra usa o mesmo eixo, então um sinal de −6 dBFS chega à altura de um slider posto
/// em −6 dB). O slider fica deslocado dentro da área (margens do tema), por isso o ponto de origem é consultado.
#[derive(Clone, Copy)]
struct MeterGeometry {
    center_x: f64,
    well_y: f64,
    well_h: f64,
    top: f64,
    bottom: f64,
}

fn meter_geometry(scale: &gtk::Scale, area: &gtk::DrawingArea) -> MeterGeometry {
    let (ox, oy) = scale
        .compute_point(area, &gtk::graphene::Point::new(0.0, 0.0))
        .map_or((0.0, 0.0), |p| (f64::from(p.x()), f64::from(p.y())));
    let rect = scale.range_rect();
    let (start, end) = scale.slider_range();
    let knob = f64::from((end - start).max(0));
    let well_y = oy + f64::from(rect.y());
    let well_h = f64::from(rect.height());
    let top = well_y + knob / 2.0;
    let bottom = (well_y + well_h - knob / 2.0).max(top);
    MeterGeometry {
        center_x: ox + f64::from(rect.x()) + f64::from(rect.width()) / 2.0,
        well_y,
        well_h,
        top,
        bottom,
    }
}

fn build_meter_area(scale: &gtk::Scale, view: &Rc<Cell<MeterView>>) -> gtk::DrawingArea {
    // puramente decorativo para tecnologias assistivas: o valor e o estado já estão nos controles
    let area = gtk::DrawingArea::builder()
        .accessible_role(gtk::AccessibleRole::Presentation)
        .vexpand(true)
        .build();
    area.set_size_request(30, 200);
    area.set_can_target(false);
    let (scale, view) = (scale.clone(), view.clone());
    area.set_draw_func(move |area, cr, _, _| {
        draw_meter(cr, &meter_geometry(&scale, area), view.get())
    });
    area
}

/// Desenha o poço, o nível (degradê fixo: verde → amarelo → vermelho), o pico mantido e o indicador de clipe.
/// O clipe e o pico têm forma própria (bloco no topo, traço claro): o estado não depende só da cor.
fn draw_meter(cr: &gtk::cairo::Context, g: &MeterGeometry, v: MeterView) {
    let MeterGeometry {
        center_x,
        well_y,
        well_h,
        top,
        bottom,
    } = *g;
    let bar_w = 10.0;
    let x = (center_x - bar_w / 2.0).floor();
    let rounded = |cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64| {
        let r = r.min(w / 2.0).min(h / 2.0);
        cr.new_sub_path();
        cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
        cr.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
        cr.arc(
            x + r,
            y + h - r,
            r,
            std::f64::consts::FRAC_PI_2,
            std::f64::consts::PI,
        );
        cr.arc(
            x + r,
            y + r,
            r,
            std::f64::consts::PI,
            3.0 * std::f64::consts::FRAC_PI_2,
        );
        cr.close_path();
    };
    // poço
    cr.set_source_rgb(0.122, 0.227, 0.271);
    rounded(cr, x, well_y, bar_w, well_h, 5.0);
    let _ = cr.fill();
    let span = bottom - top;
    if span <= 0.0 {
        return;
    }
    let y_of = |p: f32| bottom - span * f64::from(p.clamp(0.0, 1.0));
    // nível: recorta no poço e preenche de baixo até a altura do nível com o degradê fixo
    if v.level > 0.0 {
        let _ = cr.save();
        rounded(cr, x, well_y, bar_w, well_h, 5.0);
        cr.clip();
        let grad = gtk::cairo::LinearGradient::new(0.0, bottom, 0.0, top);
        grad.add_color_stop_rgb(0.0, 0.208, 0.788, 0.561);
        grad.add_color_stop_rgb(0.7, 0.333, 0.863, 0.494);
        grad.add_color_stop_rgb(0.85, 0.949, 0.831, 0.369);
        grad.add_color_stop_rgb(1.0, 0.937, 0.353, 0.373);
        let _ = cr.set_source(&grad);
        let y = y_of(v.level);
        cr.rectangle(x, y, bar_w, well_y + well_h - y);
        let _ = cr.fill();
        let _ = cr.restore();
    }
    // pico mantido: traço claro de 2 px
    if v.hold > 0.0 {
        cr.set_source_rgb(0.91, 0.949, 0.957);
        cr.rectangle(x, y_of(v.hold) - 1.0, bar_w, 2.0);
        let _ = cr.fill();
    }
    // clipe: bloco vermelho no alto do poço
    if v.clip {
        cr.set_source_rgb(0.937, 0.353, 0.373);
        rounded(cr, x, well_y, bar_w, 5.0, 2.0);
        let _ = cr.fill();
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
    /// Lista de etiquetas de aplicativos (MASTER = Não atribuídos; canais = aplicativos do canal); não existe no MIC.
    apps_box: Option<gtk::Box>,
    chip_sig: RefCell<String>,
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

fn manage_menu(id: &str, name: &str, others: &[(String, String)], emit: &Emit) -> gtk::MenuButton {
    let pop = gtk::Popover::new();
    let bx = gtk::Box::new(gtk::Orientation::Vertical, 6);
    bx.set_margin_top(8);
    bx.set_margin_bottom(8);
    bx.set_margin_start(8);
    bx.set_margin_end(8);
    let entry = gtk::Entry::builder().text(name).max_length(64).build();
    entry.update_property(&[gtk::accessible::Property::Label("Novo nome do canal")]);
    let rename = gtk::Button::with_label("Renomear");
    let remove = gtk::Button::with_label("Remover canal");
    remove.add_css_class("destructive-action");
    // spec 8.3: quem remove escolhe para onde vão as regras (outro canal ou Não atribuídos); nunca se assume
    let mut labels = vec!["Não atribuídos".to_owned()];
    labels.extend(others.iter().map(|(_, n)| n.clone()));
    let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let dest = gtk::DropDown::from_strings(&label_refs);
    dest.update_property(&[gtk::accessible::Property::Label(
        "Destino dos aplicativos deste canal ao remover",
    )]);
    bx.append(&entry);
    bx.append(&rename);
    bx.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    bx.append(&label(
        "Ao remover, os aplicativos vão para:",
        "strip-caption",
    ));
    bx.append(&dest);
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
        let (others, dest) = (others.to_vec(), dest.clone());
        remove.connect_clicked(move |b| {
            if b.label().as_deref() == Some("Confirmar remoção") {
                let pick = dest.selected() as usize;
                let destination = if pick == 0 {
                    None
                } else {
                    others.get(pick - 1).map(|(i, _)| i.clone())
                };
                e(EditCommand::RemoveChannel {
                    channel: id.clone(),
                    destination,
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
    mb.update_property(&[gtk::accessible::Property::Label(&format!(
        "Configurações do canal {name}"
    ))]);
    mb
}

/// Medidor que alimenta o slider de uma coluna: canal, MASTER ou MIC, escuta ou transmissão.
fn meter_key(kind: &ColumnKind, id: &str, transmission: bool) -> MeterKey {
    match kind {
        ColumnKind::Channel => MeterKey::Channel {
            id: id.to_owned(),
            transmission,
        },
        ColumnKind::Master => MeterKey::Master { transmission },
        ColumnKind::Mic => MeterKey::Mic { transmission },
    }
}

fn build_column(v: &ColumnView, emit: &Emit, updating: &Rc<Cell<bool>>, ctx: &AppsCtx) -> Column {
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
    if v.kind == ColumnKind::Master {
        head.append(&ctx.device_button);
    }
    if v.kind == ColumnKind::Channel {
        let others: Vec<(String, String)> = ctx
            .channels
            .borrow()
            .iter()
            .filter(|(i, _)| *i != v.id)
            .cloned()
            .collect();
        head.append(&manage_menu(&v.id, &v.title, &others, emit));
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
        input.add_css_class("hstrip");
        input.set_draw_value(false);
        input.update_property(&[gtk::accessible::Property::Label("MIC — ganho de entrada")]);
        set_gain_steps(&input);
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
            "MIC",
            "APLICATIVOS",
            "audio-input-microphone-symbolic",
            true,
            MeterKey::MicApps,
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
        &v.title,
        "ESCUTA",
        "audio-headphones-symbolic",
        false,
        meter_key(&v.kind, &v.id, false),
        updating,
        kind_handlers(&v.kind, &v.id, SendKind::Personal, emit),
    );
    let transmission = build_strip(
        &v.title,
        "TRANSMISSÃO",
        "network-wireless-symbolic",
        true,
        meter_key(&v.kind, &v.id, true),
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
    let apps_box = (v.kind != ColumnKind::Mic).then(|| {
        let title = if v.kind == ColumnKind::Master {
            "NÃO ATRIBUÍDOS"
        } else {
            "APLICATIVOS"
        };
        root.append(&label(title, "apps-title"));
        let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let scroller = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .min_content_height(84)
            .max_content_height(150)
            .propagate_natural_height(true)
            .build();
        root.append(&scroller);
        // soltar um aplicativo nesta coluna o associa a este canal (MASTER = volta a Não atribuídos)
        let target = gtk::DropTarget::new(String::static_type(), gdk::DragAction::MOVE);
        let (ctx2, channel) = (
            ctx.clone(),
            (v.kind == ColumnKind::Channel).then(|| v.id.clone()),
        );
        let r_drop = root.clone();
        target.connect_drop(move |_, value, _, _| {
            // numa soltura o `leave` não dispara: o realce sai aqui (um segundo handler nunca rodaria, o primeiro devolve true)
            r_drop.remove_css_class("drop-target");
            let Ok(key) = value.get::<String>() else {
                return false;
            };
            let app = ctx2
                .apps
                .borrow()
                .iter()
                .find(|a| a.key.as_deref() == Some(key.as_str()))
                .cloned();
            match app.and_then(|a| move_app(&a, channel.as_deref(), ctx2.session_only.get())) {
                Some(cmd) => {
                    (ctx2.emit)(cmd);
                    true
                }
                None => false,
            }
        });
        let r = root.clone();
        target.connect_enter(move |_, _, _| {
            r.add_css_class("drop-target");
            gdk::DragAction::MOVE
        });
        let r = root.clone();
        target.connect_leave(move |_| r.remove_css_class("drop-target"));
        root.add_controller(target);
        list
    });
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
        apps_box,
        chip_sig: RefCell::new(String::new()),
    };
    col.apply(v);
    col
}

/// Etiqueta de um aplicativo: clique abre o menu "Mover para…" (alternativa por teclado ao arrastar); arrastar a move.
fn build_chip(chip: &AppChip, ctx: &AppsCtx) -> gtk::MenuButton {
    let text = if chip.glyph.is_empty() {
        chip.app.display.clone()
    } else {
        format!("{} {}", chip.app.display, chip.glyph)
    };
    let pop = gtk::Popover::new();
    let bx = gtk::Box::new(gtk::Orientation::Vertical, 4);
    for set in [
        gtk::Widget::set_margin_top,
        gtk::Widget::set_margin_bottom,
        gtk::Widget::set_margin_start,
        gtk::Widget::set_margin_end,
    ] {
        set(bx.upcast_ref(), 8);
    }
    bx.append(&label(&chip.tooltip, "strip-caption"));
    let session = gtk::CheckButton::with_label("Somente nesta sessão");
    session.set_active(ctx.session_only.get());
    let mut targets: Vec<(Option<String>, String)> = ctx
        .channels
        .borrow()
        .iter()
        .map(|(id, name)| (Some(id.clone()), name.clone()))
        .collect();
    targets.push((None, "Não atribuídos".to_owned()));
    for (channel, name) in targets {
        let b = gtk::Button::with_label(&format!("Mover para {name}"));
        b.add_css_class("flat-btn");
        b.set_sensitive(chip.app.channel != channel || chip.app.source == AppSource::Session);
        let (app, emit, pop2, session2) = (
            chip.app.clone(),
            ctx.emit.clone(),
            pop.clone(),
            session.clone(),
        );
        b.connect_clicked(move |_| {
            if let Some(cmd) = move_app(&app, channel.as_deref(), session2.is_active()) {
                emit(cmd);
            }
            pop2.popdown();
        });
        bx.append(&b);
    }
    bx.append(&session);
    if chip.app.source == AppSource::Session {
        let b = gtk::Button::with_label("Reaplicar regra do perfil");
        b.add_css_class("flat-btn");
        let (app, emit, pop2) = (chip.app.clone(), ctx.emit.clone(), pop.clone());
        b.connect_clicked(move |_| {
            if let Some(cmd) = reapply_rule(&app) {
                emit(cmd);
            }
            pop2.popdown();
        });
        bx.append(&b);
    }
    pop.set_child(Some(&bx));
    let mb = gtk::MenuButton::builder()
        .label(&text)
        .popover(&pop)
        .tooltip_text(&chip.tooltip)
        .always_show_arrow(false)
        .build();
    mb.add_css_class("chip");
    if chip.temporary {
        mb.add_css_class("temp");
    }
    if chip.idle {
        mb.add_css_class("idle");
    }
    if matches!(
        chip.app.state,
        AppState::NotApplied | AppState::DontMove | AppState::Unmanaged
    ) {
        mb.add_css_class("attention");
    }
    mb.update_property(&[gtk::accessible::Property::Label(&chip.tooltip)]);
    mb.set_halign(gtk::Align::Start);
    if let Some(key) = chip.app.key.clone() {
        // fase de captura: o botão da etiqueta reivindica o gesto ao ser pressionado; só assim o arrasto o vê primeiro
        // (e só reivindica depois de passar o limiar de movimento, então o clique que abre o menu segue funcionando)
        let src = gtk::DragSource::builder()
            .actions(gdk::DragAction::MOVE)
            .propagation_phase(gtk::PropagationPhase::Capture)
            .build();
        src.connect_prepare(move |_, _, _| Some(gdk::ContentProvider::for_value(&key.to_value())));
        mb.add_controller(src);
    }
    mb
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
    profile: gtk::MenuButton,
    /// Lista de perfis dentro do menu do cabeçalho, refeita só quando muda.
    profile_list: gtk::Box,
    profile_sig: RefCell<String>,
    /// Id do perfil ativo (para "duplicar/renomear o ativo").
    active_profile: Rc<RefCell<String>>,
    emit_ui: EmitUi,
    devices: DevicePicker,
    profile_pop: gtk::Popover,
    /// Último retrato recebido (para refazer o menu de perfis ao fechá-lo).
    last_state: RefCell<Option<State>>,
    row: gtk::Box,
    add_button: gtk::MenuButton,
    chatmix: ChatMixBar,
    cols: RefCell<Vec<Column>>,
    updating: Rc<Cell<bool>>,
    existing: Rc<RefCell<Vec<String>>>,
    emit: Emit,
    ctx: AppsCtx,
    transient: RefCell<Option<String>>,
    /// Estado de exibição dos medidores ao vivo e o relógio que os deixa decair quando não chega notícia.
    meters: RefCell<MeterBoard>,
    meter_timer: Cell<bool>,
    /// A janela está visível (não minimizada)? Só então os medidores ficam ligados no serviço.
    meters_wanted: Cell<bool>,
    /// Ainda não houve retrato desta conexão com o serviço: o pedido de medidores precisa ser refeito quando ele chegar.
    needs_meter_request: Cell<bool>,
}

impl Ui {
    /// Leva o que o `MeterBoard` sabe para as barras (só redesenha as que mudaram).
    fn paint_meters(&self) {
        let now = Instant::now();
        let board = self.meters.borrow();
        for c in self.cols.borrow().iter() {
            for s in [&c.personal, &c.transmission]
                .into_iter()
                .chain(c.mic.iter().map(|m| &m.apps))
            {
                let v = board.view(&s.meter_key, now);
                if s.meter_view.get() != v {
                    s.meter_view.set(v);
                    s.meter_area.queue_draw();
                }
            }
        }
    }

    fn feed_levels(self: &Rc<Self>, levels: &[(String, f64)]) {
        self.meters.borrow_mut().feed(levels, Instant::now());
        self.paint_meters();
        self.keep_meters_decaying();
    }

    /// Sem pacotes novos (silêncio: o serviço para de enviar) as barras ainda precisam cair e soltar o pico: um relógio
    /// de 50 ms roda só enquanto houver algo para animar e se desliga sozinho.
    fn keep_meters_decaying(self: &Rc<Self>) {
        if self.meter_timer.get() || self.meters.borrow().is_idle(Instant::now()) {
            return;
        }
        self.meter_timer.set(true);
        let weak = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_millis(50), move || {
            let Some(ui) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let now = Instant::now();
            ui.meters.borrow_mut().tick(now);
            ui.paint_meters();
            if ui.meters.borrow().is_idle(now) {
                ui.meter_timer.set(false);
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    /// Recurso de desenvolvimento: o que cada barra mostra agora, em texto (`chave nível pico clipe`, posições 0..1).
    fn dump_meters(&self) -> String {
        let now = Instant::now();
        let board = self.meters.borrow();
        let mut lines = Vec::new();
        for c in self.cols.borrow().iter() {
            for s in [&c.personal, &c.transmission]
                .into_iter()
                .chain(c.mic.iter().map(|m| &m.apps))
            {
                let v = board.view(&s.meter_key, now);
                lines.push(format!(
                    "{} {:.3} {:.3} {}",
                    s.meter_key.encode(),
                    v.level,
                    v.hold,
                    v.clip
                ));
            }
        }
        lines.join("\n") + "\n"
    }

    /// Serviço sumiu ou medidores desligados: as barras voltam ao piso na hora.
    fn reset_meters(&self) {
        self.meters.borrow_mut().clear();
        self.paint_meters();
    }

    fn request_meters(&self, on: bool) {
        self.meters_wanted.set(on);
        (self.emit_ui)(UiCommand::Meters(on));
        if !on {
            self.reset_meters();
        }
    }

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

    /// Refaz as linhas do menu de perfis só quando a lista (ou o ativo) mudou.
    fn refresh_profile_menu(&self, state: &State) {
        let rows = profile_rows(state);
        let sig = format!("{rows:?}");
        if *self.profile_sig.borrow() == sig {
            return;
        }
        *self.profile_sig.borrow_mut() = sig;
        while let Some(c) = self.profile_list.first_child() {
            self.profile_list.remove(&c);
        }
        for r in rows {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            let pick = gtk::Button::with_label(&r.label);
            pick.add_css_class("flat-btn");
            pick.set_hexpand(true);
            pick.set_sensitive(r.can_switch);
            {
                let (e, id, pop) = (self.emit_ui.clone(), r.id.clone(), self.profile_pop.clone());
                pick.connect_clicked(move |_| {
                    e(UiCommand::Profile(ProfileOp::Switch(id.clone())));
                    pop.popdown(); // escolher um perfil fecha o menu
                });
            }
            line.append(&pick);
            let trash = gtk::Button::from_icon_name("user-trash-symbolic");
            trash.add_css_class("flat-btn");
            trash.set_sensitive(r.can_delete);
            trash.set_tooltip_text(Some("Excluir (vai para a lixeira, recuperável)"));
            trash.update_property(&[gtk::accessible::Property::Label(&format!(
                "Excluir o perfil {}",
                r.name
            ))]);
            {
                // exclusão em duas etapas: o primeiro clique pede confirmação (e o nome acessível diz o que será excluído);
                // fechar o menu desfaz a confirmação pendente (ver `profile_pop.connect_closed`)
                let (e, id, name) = (self.emit_ui.clone(), r.id.clone(), r.name.clone());
                trash.connect_clicked(move |b| {
                    if b.has_css_class("armed") {
                        e(UiCommand::Profile(ProfileOp::Delete(id.clone())));
                    } else {
                        // armado: ícone de aviso, borda vermelha e nome acessível de confirmação (o rótulo visual não muda,
                        // porque trocá-lo faz o GTK sobrescrever o nome acessível)
                        b.add_css_class("armed");
                        b.set_icon_name("dialog-warning-symbolic");
                        b.set_tooltip_text(Some("Clique de novo para excluir"));
                        b.update_property(&[gtk::accessible::Property::Label(&format!(
                            "Confirmar a exclusão do perfil {name}"
                        ))]);
                    }
                });
            }
            line.append(&trash);
            self.profile_list.append(&line);
        }
    }

    /// Refaz as etiquetas só das colunas cuja lista mudou (reconstruir com um menu aberto o fecharia).
    fn refresh_chips(&self, state: &State) {
        let by = chips_by_column(state);
        for col in self.cols.borrow().iter() {
            let Some(list) = &col.apps_box else { continue };
            let key = if col.kind == ColumnKind::Master {
                ""
            } else {
                col.id.as_str()
            };
            let chips = by.get(key).cloned().unwrap_or_default();
            let sig = format!(
                "{:?}",
                chips
                    .iter()
                    .map(|c| (&c.app, c.glyph, c.temporary, c.idle))
                    .collect::<Vec<_>>()
            );
            if *col.chip_sig.borrow() == sig {
                continue;
            }
            *col.chip_sig.borrow_mut() = sig;
            while let Some(c) = list.first_child() {
                list.remove(&c);
            }
            if chips.is_empty() {
                list.append(&label("nenhum aplicativo", "apps-empty"));
            }
            for c in &chips {
                list.append(&build_chip(c, &self.ctx));
            }
        }
    }

    fn apply_state(&self, state: &State, unavailable: Option<&str>) {
        self.updating.set(true);
        *self.existing.borrow_mut() = state
            .profile
            .channels
            .iter()
            .map(|c| c.id.clone())
            .collect();
        *self.last_state.borrow_mut() = Some(state.clone());
        self.profile.set_label(&active_profile_label(state));
        *self.active_profile.borrow_mut() = state.profile.id.clone();
        self.refresh_profile_menu(state);
        self.devices.refresh(state);
        *self.ctx.apps.borrow_mut() = state.apps.clone();
        *self.ctx.channels.borrow_mut() = state
            .profile
            .channels
            .iter()
            .map(|c| (c.id.clone(), c.name.clone()))
            .collect();
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
                .map(|v| build_column(v, &self.emit, &self.updating, &self.ctx))
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
        self.refresh_chips(state);
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
        self.paint_meters();
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
/// Níveis de exemplo para o modo `--demo`: um mix em andamento, com o MASTER perto do topo.
fn demo_levels() -> Vec<(String, f64)> {
    [
        ("ch:game:personal", 0.42),
        ("ch:game:transmission", 0.42),
        ("ch:chat:personal", 0.16),
        ("ch:chat:transmission", 0.16),
        ("ch:media:personal", 0.30),
        ("ch:media:transmission", 0.30),
        ("ch:aux:personal", 0.0),
        ("master:personal", 0.62),
        ("master:transmission", 0.40),
        ("mic:personal", 0.0),
        ("mic:transmission", 0.12),
        ("mic-apps", 0.10),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect()
}

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
        E::SetPreferredOutput(Some("alsa_output.fone".into())),
        E::SetPreferredMicrophone(Some("alsa_input.usb-microfone-desconectado".into())),
    ] {
        p = apply(&p, &cmd).expect("demo").0;
    }
    let app = |name: &str, bin: &str, ch: Option<&str>, src: AppSource, st: AppState| AppEntry {
        key: Some(format!("bin:{bin}")),
        display: name.into(),
        app_id: None,
        binary: Some(bin.into()),
        name: Some(name.into()),
        channel: ch.map(Into::into),
        source: src,
        state: st,
        streams: 1,
    };
    State {
        serial: 1,
        profile: p,
        connected: true,
        persist_error: None,
        absent_devices: vec!["alsa_output.usb-fone-exemplo".into()],
        reconnect_attempts: 0,
        default_output: DefaultOutput::Active,
        devices: vec![
            iara_ipc::DeviceEntry {
                key: "alsa_output.fone".into(),
                description: "Fone (saída analógica)".into(),
                output: true,
            },
            iara_ipc::DeviceEntry {
                key: "alsa_output.hdmi".into(),
                description: "Monitor HDMI".into(),
                output: true,
            },
            iara_ipc::DeviceEntry {
                key: "alsa_input.fifine".into(),
                description: "fifine AM8 Pro Mono".into(),
                output: false,
            },
            iara_ipc::DeviceEntry {
                key: "alsa_input.webcam".into(),
                description: "Webcam C922".into(),
                output: false,
            },
        ],
        profiles: vec![
            iara_ipc::ProfileEntry {
                id: "default".into(),
                name: "Padrão".into(),
                readable: true,
            },
            iara_ipc::ProfileEntry {
                id: "streaming".into(),
                name: "Streaming".into(),
                readable: true,
            },
            iara_ipc::ProfileEntry {
                id: "jogos".into(),
                name: "Só jogos".into(),
                readable: true,
            },
        ],
        apps: vec![
            app(
                "Zen",
                "zen",
                Some("media"),
                AppSource::Rule,
                AppState::Applied,
            ),
            app(
                "Cider",
                "cider",
                Some("media"),
                AppSource::Rule,
                AppState::Applied,
            ),
            app(
                "Discord",
                "discord",
                Some("chat"),
                AppSource::Rule,
                AppState::Waiting,
            ),
            app(
                "Jogo",
                "jogo",
                Some("game"),
                AppSource::Rule,
                AppState::DontMove,
            ),
            app(
                "Steam",
                "steam",
                Some("game"),
                AppSource::Session,
                AppState::Applied,
            ),
            app(
                "Spotify",
                "spotify",
                None,
                AppSource::Default,
                AppState::Applied,
            ),
            app(
                "Firefox",
                "firefox",
                None,
                AppSource::Default,
                AppState::Applied,
            ),
            app(
                "Navegador",
                "navegador",
                None,
                AppSource::Default,
                AppState::Outside,
            ),
        ],
    }
}

/// Renderiza qualquer widget com superfície própria (janela, popover) em PNG, sem ferramenta de captura de tela.
fn save_widget_png(widget: &gtk::Widget, path: &std::path::Path) -> Result<(), String> {
    let paintable = gtk::WidgetPaintable::new(Some(widget));
    paintable_to_png(widget, &paintable, path)
}

fn paintable_to_png(
    widget: &gtk::Widget,
    paintable: &gtk::WidgetPaintable,
    path: &std::path::Path,
) -> Result<(), String> {
    let (w, h) = (widget.width() as f64, widget.height() as f64);
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(snapshot.upcast_ref::<gdk::Snapshot>(), w, h);
    let node = snapshot.to_node().ok_or("nada renderizado")?;
    let renderer = widget
        .native()
        .and_then(|n| n.renderer())
        .ok_or("sem renderer")?;
    let texture = renderer.render_texture(&node, None);
    texture.save_to_png(path).map_err(|e| e.to_string())
}

/// Grava a janela em PNG. Um `WidgetPaintable` recém-criado só tem conteúdo depois de o widget ser desenhado num quadro
/// seguinte: cria o paintable, força um redesenho e só então tira o retrato (com algumas tentativas).
fn shoot(window: &gtk::ApplicationWindow, path: PathBuf, tries: u32) {
    let paintable = gtk::WidgetPaintable::new(Some(window));
    window.queue_draw();
    shoot_with(window.clone(), paintable, path, tries);
}

fn shoot_with(
    window: gtk::ApplicationWindow,
    paintable: gtk::WidgetPaintable,
    path: PathBuf,
    tries: u32,
) {
    glib::timeout_add_local_once(Duration::from_millis(150), move || {
        match paintable_to_png(window.upcast_ref::<gtk::Widget>(), &paintable, &path) {
            Ok(()) => eprintln!("captura salva em {}", path.display()),
            Err(_) if tries > 0 => {
                window.queue_draw();
                shoot_with(window, paintable, path, tries - 1);
            }
            Err(e) => eprintln!("falha na captura: {e}"),
        }
    });
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
    let emit_ui: EmitUi = {
        let b = backend.clone();
        Rc::new(move |cmd| {
            if let Some(b) = b.borrow().as_ref() {
                b.send(cmd);
            }
        })
    };
    let emit: Emit = {
        let e = emit_ui.clone();
        Rc::new(move |cmd| e(UiCommand::Edit(cmd)))
    };
    let updating = Rc::new(Cell::new(false));
    let (device_button, devices) = build_device_menu(&emit, &updating);
    let ctx = AppsCtx {
        emit: emit_ui,
        apps: Rc::default(),
        channels: Rc::default(),
        session_only: Rc::new(Cell::new(false)),
        device_button,
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
    let profile_pop = gtk::Popover::new();
    let profile_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
    for set in [
        gtk::Widget::set_margin_top,
        gtk::Widget::set_margin_bottom,
        gtk::Widget::set_margin_start,
        gtk::Widget::set_margin_end,
    ] {
        set(profile_box.upcast_ref(), 10);
    }
    let profile_list = gtk::Box::new(gtk::Orientation::Vertical, 2);
    profile_box.append(&label("PERFIS", "strip-caption"));
    profile_box.append(&profile_list);
    profile_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    let profile_name = gtk::Entry::builder()
        .placeholder_text("Nome do perfil")
        .max_length(64)
        .build();
    profile_name.update_property(&[gtk::accessible::Property::Label(
        "Nome do perfil para criar, duplicar ou renomear",
    )]);
    let (btn_new, btn_dup, btn_ren) = (
        gtk::Button::with_label("Novo perfil"),
        gtk::Button::with_label("Duplicar o ativo"),
        gtk::Button::with_label("Renomear o ativo"),
    );
    profile_box.append(&profile_name);
    for b in [&btn_new, &btn_dup, &btn_ren] {
        b.add_css_class("flat-btn");
        profile_box.append(b);
    }
    profile_pop.set_child(Some(&profile_box));
    let active_profile: Rc<RefCell<String>> = Rc::default();
    for (btn, kind) in [(&btn_new, 0), (&btn_dup, 1), (&btn_ren, 2)] {
        let (e, entry, pop, active) = (
            ctx.emit.clone(),
            profile_name.clone(),
            profile_pop.clone(),
            active_profile.clone(),
        );
        btn.connect_clicked(move |_| {
            let name = entry.text().trim().to_owned();
            if name.is_empty() {
                return;
            }
            let id = active.borrow().clone();
            e(UiCommand::Profile(match kind {
                0 => ProfileOp::Create(name),
                1 => ProfileOp::Duplicate { id, name },
                _ => ProfileOp::Rename { id, name },
            }));
            entry.set_text("");
            pop.popdown();
        });
    }
    let profile = gtk::MenuButton::builder()
        .label("Perfil ▾")
        .popover(&profile_pop)
        .tooltip_text("Trocar, criar, duplicar ou renomear perfis")
        .always_show_arrow(false)
        .build();
    profile.add_css_class("profile-button");
    profile.update_property(&[gtk::accessible::Property::Label("Menu de perfis")]);
    title.append(&profile);
    header.set_title_widget(Some(&title));
    // arrastar ou mover um aplicativo salva uma regra; com este interruptor vale só nesta sessão (spec 8.1)
    let session_toggle = gtk::ToggleButton::with_label("Mover só nesta sessão");
    session_toggle.set_tooltip_text(Some(
        "Ligado: mover um aplicativo vale só até ele parar de tocar, sem salvar a regra no perfil",
    ));
    {
        let only = ctx.session_only.clone();
        session_toggle.connect_toggled(move |b| only.set(b.is_active()));
    }
    header.pack_end(&session_toggle);
    // “Desligar mixer / voltar ao áudio normal” (spec 14): restaura a saída padrão anterior e encerra o serviço; confirma antes
    let off_pop = gtk::Popover::new();
    let off_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
    for set in [
        gtk::Widget::set_margin_top,
        gtk::Widget::set_margin_bottom,
        gtk::Widget::set_margin_start,
        gtk::Widget::set_margin_end,
    ] {
        set(off_box.upcast_ref(), 10);
    }
    let off_text = label(
        "O áudio volta ao normal: a saída padrão anterior é restaurada\ne o serviço do Iara é encerrado.",
        "strip-caption",
    );
    off_text.set_xalign(0.0);
    let off_ok = gtk::Button::with_label("Desligar mixer");
    off_ok.add_css_class("destructive-action");
    off_box.append(&off_text);
    off_box.append(&off_ok);
    off_pop.set_child(Some(&off_box));
    {
        let (e, p) = (ctx.emit.clone(), off_pop.clone());
        off_ok.connect_clicked(move |_| {
            e(UiCommand::Deactivate);
            p.popdown();
        });
    }
    let off_button = gtk::MenuButton::builder()
        .label("Desligar mixer")
        .popover(&off_pop)
        .tooltip_text("Voltar ao áudio normal e encerrar o serviço")
        .build();
    header.pack_end(&off_button);
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
    scale.add_css_class("hstrip");
    scale.set_hexpand(true);
    scale.set_draw_value(false);
    scale.add_mark(0.0, gtk::PositionType::Bottom, None);
    scale.adjustment().set_step_increment(0.05);
    scale.adjustment().set_page_increment(0.25);
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
    name_entry.update_property(&[gtk::accessible::Property::Label("Nome do novo canal")]);
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

    let ctx_device_button = ctx.device_button.clone();
    let profile_button = profile.clone();
    let ui = Rc::new(Ui {
        window: window.clone(),
        banner,
        profile,
        profile_list,
        profile_sig: RefCell::new(String::new()),
        active_profile,
        emit_ui: ctx.emit.clone(),
        devices,
        profile_pop: profile_pop.clone(),
        last_state: RefCell::new(None),
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
        ctx,
        transient: RefCell::new(None),
        meters: RefCell::new(MeterBoard::default()),
        meter_timer: Cell::new(false),
        meters_wanted: Cell::new(true),
        needs_meter_request: Cell::new(true),
    });
    UI.with(|u| *u.borrow_mut() = Some(ui.clone()));
    {
        // fechar o menu de perfis desfaz uma confirmação de exclusão pendente: refaz as linhas a partir do último retrato
        let weak = Rc::downgrade(&ui);
        profile_pop.connect_closed(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.profile_sig.borrow_mut().clear();
                let last = ui.last_state.borrow().clone();
                if let Some(state) = last {
                    ui.refresh_profile_menu(&state);
                }
            }
        });
    }

    if opts.demo {
        ui.apply_state(&demo_state(), None);
        // níveis fixos para a captura de tela e para ver o desenho sem serviço (não decaem: nenhum relógio roda)
        ui.meters.borrow_mut().feed(&demo_levels(), Instant::now());
        ui.paint_meters();
    } else {
        // enquanto o primeiro retrato não chega, mostra que está procurando o serviço
        ui.render_banner(None, Some("procurando…"));
        let cb: OnUpdate = std::sync::Arc::new(|u: Update| {
            on_main(move |ui| match u {
                Update::State(st) => {
                    *ui.transient.borrow_mut() = None;
                    ui.apply_state(&st, None);
                    // o serviço (re)apareceu: o pedido de medidores morre com a conexão antiga, então se refaz
                    if ui.needs_meter_request.replace(false) && ui.meters_wanted.get() {
                        (ui.emit_ui)(UiCommand::Meters(true));
                    }
                }
                Update::Levels(levels) => ui.feed_levels(&levels),
                Update::Unavailable(m) => {
                    ui.needs_meter_request.set(true);
                    ui.reset_meters();
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
        // oculta ou minimizada, a janela não precisa de medidores: desliga no serviço e religa ao voltar. Cobre as duas formas:
        // X11 desmapeia a janela ao minimizar; no Wayland ela continua mapeada e só muda o estado do toplevel.
        let set_visible = {
            let weak = Rc::downgrade(&ui);
            Rc::new(move |visible: bool| {
                if let Some(ui) = weak.upgrade() {
                    if visible != ui.meters_wanted.get() {
                        ui.request_meters(visible);
                    }
                }
            })
        };
        {
            let (on, off) = (set_visible.clone(), set_visible.clone());
            window.connect_map(move |_| on(true));
            window.connect_unmap(move |_| off(false));
        }
        window.connect_realize(move |w| {
            let Some(top) = w.surface().and_then(|s| s.downcast::<gdk::Toplevel>().ok()) else {
                return;
            };
            let set_visible = set_visible.clone();
            top.connect_state_notify(move |t| {
                set_visible(!t.state().contains(gdk::ToplevelState::MINIMIZED));
            });
        });
    }

    window.present();
    // recurso de desenvolvimento: `kill -USR1 <pid>` grava a janela em $IARA_UI_SHOT_PATH, sem encerrar o app
    if let Some(path) = std::env::var_os("IARA_UI_SHOT_PATH").map(PathBuf::from) {
        let w = window.clone();
        let path_main = path.clone();
        glib::unix_signal_add_local(10, move || {
            UI.with(|u| {
                if let Some(ui) = u.borrow().as_ref() {
                    let _ = std::fs::write(path_main.with_extension("meters"), ui.dump_meters());
                }
            });
            shoot(&w, path_main.clone(), 8);
            glib::ControlFlow::Continue
        });
        // `kill -USR2`: grava os popovers abertos (menus de perfis e de dispositivos) ao lado, com sufixo
        let (menus, base) = (
            vec![
                ("dispositivos", ctx_device_button.clone()),
                ("perfis", profile_button.clone()),
            ],
            path.clone(),
        );
        glib::unix_signal_add_local(12, move || {
            for (name, button) in &menus {
                if let Some(pop) = button.popover().filter(|p| p.is_visible()) {
                    let out = base.with_file_name(format!("popover-{name}.png"));
                    match save_widget_png(pop.upcast_ref::<gtk::Widget>(), &out) {
                        Ok(()) => eprintln!("popover salvo em {}", out.display()),
                        Err(e) => eprintln!("falha no popover {name}: {e}"),
                    }
                }
            }
            glib::ControlFlow::Continue
        });
    }
    if let Some(path) = opts.screenshot.clone() {
        let (w, app) = (window.clone(), app.clone());
        let delay = std::env::var("IARA_UI_SHOT_DELAY_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(900);
        glib::timeout_add_local_once(Duration::from_millis(delay), move || {
            shoot(&w, path, 8);
            // dá tempo das tentativas (até 8 × 150 ms) antes de sair
            glib::timeout_add_local_once(Duration::from_millis(2000), move || app.quit());
        });
    }
}

pub fn run(opts: Options) -> glib::ExitCode {
    // o modo de demonstração (e a captura) nunca reativa uma janela real já aberta: é sempre uma instância à parte
    let flags = if opts.demo || opts.screenshot.is_some() {
        gtk::gio::ApplicationFlags::NON_UNIQUE
    } else {
        gtk::gio::ApplicationFlags::default()
    };
    let app = gtk::Application::builder()
        .application_id(APP_ID)
        .flags(flags)
        .build();
    app.connect_activate(move |app| build_ui(app, &opts));
    // os argumentos próprios (--demo etc.) já foram lidos; não repassá-los ao GTK
    app.run_with_args::<&str>(&[])
}
