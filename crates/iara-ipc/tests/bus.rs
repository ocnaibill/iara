//! Testes contra o barramento de sessão real; sem barramento (CI sem D-Bus) eles se declaram ignorados e passam.
use iara_core::edit::{self, EditCommand, MicSend, SendKind};
use iara_core::{initial_profile, Gain, Profile};
use iara_ipc::{
    serve, AppEntry, AppSource, AppState, Client, ClientError, Controller, Server, SessionChoice,
    State,
};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct Fake {
    inner: Mutex<(Profile, u64)>,
    notifier: Mutex<Option<iara_ipc::Notifier>>,
    sessions: Mutex<Vec<(String, SessionChoice)>>,
}

impl Controller for Fake {
    fn state(&self) -> Result<State, String> {
        let g = self.inner.lock().unwrap();
        Ok(State {
            serial: g.1,
            profile: g.0.clone(),
            connected: true,
            persist_error: Some("disco cheio".into()),
            absent_devices: vec!["fone".into()],
            reconnect_attempts: 3,
            apps: vec![AppEntry {
                key: Some("bin:zen".into()),
                display: "Zen".into(),
                app_id: Some("app.zen_browser.zen".into()),
                binary: Some("zen".into()),
                name: None,
                channel: Some("media".into()),
                source: AppSource::Rule,
                state: AppState::DontMove,
                streams: 2,
            }],
        })
    }

    fn session_choice(&self, key: String, choice: SessionChoice) -> Result<u64, String> {
        if key.is_empty() {
            return Err("chave vazia".into());
        }
        self.sessions.lock().unwrap().push((key, choice));
        let mut g = self.inner.lock().unwrap();
        g.1 += 1;
        Ok(g.1)
    }

    fn edit(&self, cmd: EditCommand) -> Result<u64, String> {
        let serial = {
            let mut g = self.inner.lock().unwrap();
            let (next, _) = edit::apply(&g.0, &cmd).map_err(|e| e.to_string())?;
            g.0 = next;
            g.1 += 1;
            g.1
        };
        if let Some(n) = self.notifier.lock().unwrap().as_ref() {
            n(serial);
        }
        Ok(serial)
    }
}

static N: AtomicU32 = AtomicU32::new(0);

fn bus_available() -> bool {
    zbus::blocking::Connection::session().is_ok()
}

fn start() -> Option<(String, Arc<Fake>, Server)> {
    if !bus_available() {
        eprintln!("sem barramento de sessão: teste ignorado");
        return None;
    }
    let name = format!(
        "dev.iara.Test{}x{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    );
    let fake = Arc::new(Fake {
        inner: Mutex::new((initial_profile(), 1)),
        notifier: Mutex::new(None),
        sessions: Mutex::new(Vec::new()),
    });
    let server = serve(&name, fake.clone()).expect("serve");
    *fake.notifier.lock().unwrap() = Some(server.notifier());
    Some((name, fake, server))
}

#[test]
fn state_round_trips_the_whole_profile_and_status() {
    let Some((name, fake, _server)) = start() else {
        return;
    };
    fake.edit(EditCommand::SetChannelGain {
        channel: "game".into(),
        send: SendKind::Personal,
        gain: Gain::SILENCE,
    })
    .unwrap();
    let state = Client::connect(name).unwrap().state().unwrap();
    assert_eq!(state.serial, 2);
    assert_eq!(state.profile, fake.state().unwrap().profile);
    assert_eq!(state.profile.channels[0].personal.gain, Gain::SILENCE);
    assert_eq!(state.persist_error.as_deref(), Some("disco cheio"));
    assert_eq!(
        (
            state.connected,
            state.absent_devices,
            state.reconnect_attempts
        ),
        (true, vec!["fone".to_owned()], 3)
    );
}

#[test]
fn every_edit_command_works_over_the_wire_and_matches_the_local_result() {
    let Some((name, fake, _server)) = start() else {
        return;
    };
    let client = Client::connect(name).unwrap();
    let g = Gain::from_db(-6.0).unwrap();
    let cmds = [
        EditCommand::SetChannelGain {
            channel: "game".into(),
            send: SendKind::Personal,
            gain: g,
        },
        EditCommand::SetChannelMute {
            channel: "chat".into(),
            send: SendKind::Transmission,
            muted: true,
        },
        EditCommand::SetChannelEnabled {
            channel: "aux".into(),
            send: SendKind::Transmission,
            enabled: true,
        },
        EditCommand::SetMasterGain {
            send: SendKind::Transmission,
            gain: g,
        },
        EditCommand::SetMasterMute {
            send: SendKind::Personal,
            muted: true,
        },
        EditCommand::SetMicGlobalMute(true),
        EditCommand::SetMicInputGain(Gain::SILENCE),
        EditCommand::SetMicSendGain {
            send: MicSend::Applications,
            gain: g,
        },
        EditCommand::SetMicSendMute {
            send: MicSend::Transmission,
            muted: true,
        },
        EditCommand::SetMicSendEnabled {
            send: MicSend::Personal,
            enabled: true,
        },
        EditCommand::SetChatMixPosition(-0.5),
        EditCommand::SetChatMixChannels(Some(("media".into(), "aux".into()))),
        EditCommand::SetPreferredOutput(Some("alsa_output.fone".into())),
        EditCommand::SetPreferredMicrophone(Some("alsa_input.mic".into())),
        EditCommand::AddChannel {
            id: "musica".into(),
            name: "Música".into(),
        },
        EditCommand::RenameChannel {
            channel: "musica".into(),
            name: "Sons".into(),
        },
        EditCommand::RemoveChannel {
            channel: "musica".into(),
            destination: None,
        },
        EditCommand::SetPreferredOutput(None),
        EditCommand::SetChatMixChannels(None),
        EditCommand::AssignApp {
            matcher: iara_core::apps::AppMatcher {
                binary: Some("zen".into()),
                ..Default::default()
            },
            channel: Some("media".into()),
        },
        EditCommand::AssignApp {
            matcher: iara_core::apps::AppMatcher {
                app_id: Some("com.discordapp.Discord".into()),
                name: Some("Discord".into()),
                ..Default::default()
            },
            channel: Some("chat".into()),
        },
        EditCommand::RemoveChannel {
            channel: "chat".into(),
            destination: Some("game".into()),
        },
        EditCommand::AssignApp {
            matcher: iara_core::apps::AppMatcher {
                binary: Some("zen".into()),
                ..Default::default()
            },
            channel: None,
        },
    ];
    let mut local = initial_profile();
    let mut last = 1;
    for cmd in &cmds {
        local = edit::apply(&local, cmd).unwrap().0;
        let serial = client.edit(cmd).unwrap_or_else(|e| panic!("{cmd:?}: {e}"));
        assert!(serial > last, "a versão cresce: {serial} > {last}");
        last = serial;
    }
    assert_eq!(client.state().unwrap().profile, local);
    assert_eq!(fake.state().unwrap().profile, local);
}

#[test]
fn rejected_commands_come_back_as_errors_with_the_reason_and_change_nothing() {
    let Some((name, fake, _server)) = start() else {
        return;
    };
    let client = Client::connect(name).unwrap();
    let before = fake.state().unwrap();
    for (cmd, needle) in [
        (EditCommand::SetChatMixPosition(7.0), "ChatMix"),
        (
            EditCommand::RemoveChannel {
                channel: "fantasma".into(),
                destination: None,
            },
            "fantasma",
        ),
        (
            EditCommand::AddChannel {
                id: "../x".into(),
                name: "X".into(),
            },
            "id inválido",
        ),
    ] {
        match client.edit(&cmd) {
            Err(ClientError::Rejected(m)) => assert!(m.contains(needle), "{m}"),
            other => panic!("esperava Rejected, veio {other:?}"),
        }
    }
    assert_eq!(fake.state().unwrap(), before);
}

#[test]
fn the_changed_signal_reaches_a_subscriber_for_each_edit() {
    let Some((name, _fake, _server)) = start() else {
        return;
    };
    let client = Client::connect(name).unwrap();
    let sub = client.subscribe().unwrap();
    let a = client.edit(&EditCommand::SetMicGlobalMute(true)).unwrap();
    let b = client.edit(&EditCommand::SetMicGlobalMute(false)).unwrap();
    assert_eq!(sub.wait(Duration::from_secs(3)), Some(a));
    assert_eq!(sub.wait(Duration::from_secs(3)), Some(b));
    assert_eq!(
        sub.wait(Duration::from_millis(300)),
        None,
        "sem avisos extras"
    );
}

#[test]
fn only_one_instance_can_own_the_name_and_a_missing_service_is_reported_as_unavailable() {
    let Some((name, fake, _server)) = start() else {
        return;
    };
    assert!(
        serve(&name, fake).is_err(),
        "o segundo serviço não pode assumir o nome"
    );
    let ghost = Client::connect("dev.iara.NaoExiste").unwrap();
    assert!(matches!(ghost.state(), Err(ClientError::Unavailable(_))));
    assert!(matches!(
        ghost.edit(&EditCommand::SetMicGlobalMute(true)),
        Err(ClientError::Unavailable(_))
    ));
}

#[test]
fn apps_travel_in_the_state_and_session_choices_reach_the_service() {
    let Some((name, fake, _server)) = start() else {
        return;
    };
    let client = Client::connect(name).unwrap();
    let state = client.state().unwrap();
    assert_eq!(
        state.apps,
        fake.state().unwrap().apps,
        "a lista de aplicativos faz a viagem inteira"
    );
    assert_eq!(state.apps[0].identity().binary.as_deref(), Some("zen"));
    client
        .session_choice("bin:zen", &SessionChoice::Channel("game".into()))
        .unwrap();
    client
        .session_choice("bin:zen", &SessionChoice::Unassigned)
        .unwrap();
    client
        .session_choice("bin:zen", &SessionChoice::Clear)
        .unwrap();
    assert_eq!(
        *fake.sessions.lock().unwrap(),
        [
            ("bin:zen".to_owned(), SessionChoice::Channel("game".into())),
            ("bin:zen".to_owned(), SessionChoice::Unassigned),
            ("bin:zen".to_owned(), SessionChoice::Clear),
        ]
    );
    assert!(
        matches!(client.session_choice("", &SessionChoice::Clear), Err(ClientError::Rejected(m)) if m.contains("vazia"))
    );
}
