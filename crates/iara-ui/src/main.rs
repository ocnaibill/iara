#[cfg(feature = "gtk-ui")]
use gtk::prelude::*;

#[cfg(feature = "gtk-ui")]
fn main() -> gtk::glib::ExitCode {
    let app = gtk::Application::builder()
        .application_id("dev.iara.Mixer")
        .build();
    app.connect_activate(|app| {
        let label = gtk::Label::new(Some(&format!(
            "Iara — Mixer de áudio para Linux\nEspecificação {}\nServiço e áudio ainda não conectados.",
            iara_core::SPEC_VERSION
        )));
        let window = gtk::ApplicationWindow::builder()
            .application(app).title("Iara")
            .default_width(960).default_height(640).child(&label).build();
        window.present();
    });
    app.run()
}

#[cfg(not(feature = "gtk-ui"))]
fn main() -> std::process::ExitCode {
    eprintln!("Interface desabilitada. Use cargo run -p iara-ui --features gtk-ui");
    std::process::ExitCode::FAILURE
}
