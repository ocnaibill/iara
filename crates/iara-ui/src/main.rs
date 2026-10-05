#[cfg(feature = "gtk-ui")]
fn main() -> gtk::glib::ExitCode {
    use iara_ui::app::{run, Options};
    let mut opts = Options {
        demo: false,
        screenshot: None,
        bus_name: std::env::var("IARA_BUS_NAME").unwrap_or_else(|_| iara_ipc::BUS_NAME.to_owned()),
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--demo" => opts.demo = true,
            "--screenshot" => opts.screenshot = args.next().map(Into::into),
            "--help" | "-h" => {
                eprintln!("uso: iara-ui [--demo] [--screenshot ARQUIVO.png]\n  --demo: mostra dados de exemplo, sem serviço\n  IARA_BUS_NAME: nome do serviço no D-Bus (testes)");
                return gtk::glib::ExitCode::SUCCESS;
            }
            other => eprintln!("argumento ignorado: {other}"),
        }
    }
    run(opts)
}

#[cfg(not(feature = "gtk-ui"))]
fn main() -> std::process::ExitCode {
    eprintln!("Interface desabilitada. Use cargo run -p iara-ui --features gtk-ui");
    std::process::ExitCode::FAILURE
}
