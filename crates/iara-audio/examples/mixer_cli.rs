//! Mixer mínimo por stdin para testar o motor de ponta a ponta (perfil → plano → PipeWire).
//! Comandos: gain CANAL personal|transmission DB | mute CANAL personal|transmission true|false |
//! enable CANAL personal|transmission true|false | mic-mute true|false | mic-gain DB | chatmix X |
//! master-gain personal|transmission DB | quit
use iara_audio::Engine;
use iara_core::topology::plan;
use iara_core::{initial_profile, Gain, Profile, SendControl};
use std::io::BufRead;

fn send<'a>(p: &'a mut Profile, ch: &str, which: &str) -> Option<&'a mut SendControl> {
    let c = p.channels.iter_mut().find(|c| c.id == ch)?;
    match which {
        "personal" => Some(&mut c.personal),
        "transmission" => Some(&mut c.transmission),
        _ => None,
    }
}

fn gain(db: &str) -> Option<Gain> {
    Gain::from_db(db.parse().ok()?).ok()
}

fn main() {
    let engine = Engine::start().expect("PipeWire indisponível");
    let mut profile = initial_profile();
    let apply = |p: &Profile| match plan(p).map(|pl| engine.apply(pl)) {
        Ok(Ok(r)) => println!("ok observados={} ausentes={:?}", r.observed, r.missing),
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
