//! Mixer mínimo por stdin para testar o motor de ponta a ponta (perfil → plano → PipeWire).
//! Comandos: gain CANAL personal|transmission DB | mute CANAL personal|transmission true|false |
//! enable CANAL personal|transmission true|false | mic-mute true|false | mic-gain DB | chatmix X |
//! master-gain personal|transmission DB | output NÓ|none | mic-device NÓ|none | route CHAVE NÓ|default | events | quit
use iara_audio::{Engine, Event, RouteTarget};
use iara_core::topology::plan;
use iara_core::{initial_profile, DevicePreference, Gain, Profile, SendControl};
use std::collections::HashMap;
use std::io::BufRead;

fn send<'a>(p: &'a mut Profile, ch: &str, which: &str) -> Option<&'a mut SendControl> {
    let c = p.channels.iter_mut().find(|c| c.id == ch)?;
    match which {
        "personal" => Some(&mut c.personal),
        "transmission" => Some(&mut c.transmission),
        _ => None,
    }
}

fn pref(node: &str) -> Option<DevicePreference> {
    (node != "none").then(|| DevicePreference {
        persistent_key: node.to_owned(),
    })
}

fn gain(db: &str) -> Option<Gain> {
    Gain::from_db(db.parse().ok()?).ok()
}

fn main() {
    let engine = Engine::start().expect("PipeWire indisponível");
    let mut profile = initial_profile();
    let mut routes: HashMap<String, RouteTarget> = HashMap::new();
    let apply = |p: &Profile| match plan(p).map(|pl| engine.apply(pl)) {
        Ok(Ok(r)) => println!(
            "ok observados={} ausentes={:?} dispositivos_ausentes={:?}",
            r.observed, r.missing, r.absent_devices
        ),
        Ok(Err(e)) => println!("erro: {e}"),
        Err(e) => println!("perfil inválido: {e:?}"),
    };
    apply(&profile);
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        let w: Vec<&str> = line.split_whitespace().collect();
        let ok = match w.as_slice() {
            ["gain", ch, s, db] => match (send(&mut profile, ch, s), gain(db)) {
                (Some(x), Some(g)) => {
                    x.gain = g;
                    true
                }
                _ => false,
            },
            ["mute", ch, s, b] => send(&mut profile, ch, s)
                .map(|x| x.muted = *b == "true")
                .is_some(),
            ["enable", ch, s, b] => send(&mut profile, ch, s)
                .map(|x| x.enabled = *b == "true")
                .is_some(),
            ["mic-mute", b] => {
                profile.microphone.global_mute = *b == "true";
                true
            }
            ["mic-gain", db] => gain(db)
                .map(|g| profile.microphone.input_gain = g)
                .is_some(),
            ["chatmix", x] => x.parse().map(|v| profile.chatmix.position = v).is_ok(),
            ["master-gain", s, db] => {
                let target = match *s {
                    "personal" => &mut profile.master.personal,
                    "transmission" => &mut profile.master.transmission,
                    _ => continue,
                };
                gain(db).map(|g| target.gain = g).is_some()
            }
            ["output", node] => {
                profile.preferred_output = pref(node);
                true
            }
            ["mic-device", node] => {
                profile.preferred_microphone = pref(node);
                true
            }
            ["route", key, node] => {
                routes.insert(
                    (*key).to_owned(),
                    if *node == "default" {
                        RouteTarget::Default
                    } else {
                        RouteTarget::Node((*node).to_owned())
                    },
                );
                match engine.set_routes(routes.clone()) {
                    Ok(()) => println!("rota definida"),
                    Err(e) => println!("erro: {e}"),
                }
                continue;
            }
            ["events", ..] => {
                for e in engine.events() {
                    match e {
                        Event::Apps(apps) => {
                            for a in apps {
                                let streams: Vec<String> = a
                                    .streams
                                    .iter()
                                    .map(|s| {
                                        format!(
                                            "{}:{:?}->{}",
                                            s.node_id,
                                            s.state,
                                            s.linked_to.join("+")
                                        )
                                    })
                                    .collect();
                                println!(
                                    "app {} [{}] {:?} {}",
                                    a.identity.display_name(),
                                    a.identity.key().unwrap_or_default(),
                                    a.state,
                                    streams.join(" ")
                                );
                            }
                            println!("--");
                        }
                        other => println!("evento {other:?}"),
                    }
                }
                println!("fim-eventos");
                continue;
            }
            ["quit"] => break,
            _ => false,
        };
        if ok {
            apply(&profile);
        } else {
            println!("comando inválido: {line}");
        }
    }
}
