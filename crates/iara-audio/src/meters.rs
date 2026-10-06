//! Taps de medição: um fluxo de captura passivo por barramento (o monitor do sink nulo), que guarda o pico de amostra na
//! thread de áudio num atômico. A thread do motor lê e zera os picos a cada intervalo e os entrega como `Event::Levels`.
//!
//! O tap só existe enquanto uma interface pede medidores; sem ninguém olhando não há fluxo extra no grafo (spec 6.4.1).

use pipewire as pw;
use pw::spa;
use pw::spa::pod::Pod;
use pw::stream::{StreamFlags, StreamListener, StreamRc};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

struct Tap {
    stream: StreamRc,
    _listener: StreamListener<()>,
    peak: Arc<AtomicU32>,
}

/// Conjunto de taps ativos, por nome do barramento.
#[derive(Default)]
pub struct Taps {
    taps: HashMap<String, Tap>,
}

/// Pico (valor absoluto máximo) de um bloco de amostras f32 little-endian entrelaçadas.
pub fn peak_of(bytes: &[u8]) -> f32 {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| f32::from_le_bytes(*c).abs())
        .filter(|v| v.is_finite())
        .fold(0.0, f32::max)
}

impl Taps {
    pub fn is_empty(&self) -> bool {
        self.taps.is_empty()
    }

    /// Faz o conjunto de taps corresponder exatamente a `buses`: cria os que faltam e remove os que sobram.
    pub fn sync(&mut self, core: &pw::core::CoreRc, buses: &[String]) {
        self.taps.retain(|name, tap| {
            let keep = buses.contains(name);
            if !keep {
                let _ = tap.stream.disconnect();
            }
            keep
        });
        for bus in buses {
            if self.taps.contains_key(bus) {
                continue;
            }
            match make_tap(core, bus) {
                Ok(tap) => {
                    self.taps.insert(bus.clone(), tap);
                }
                Err(e) => eprintln!("iara-audio: tap de medição de {bus}: {e}"),
            }
        }
    }

    /// Lê e zera os picos acumulados desde a última leitura (linear, 0..).
    pub fn take_peaks(&self) -> HashMap<String, f32> {
        self.taps
            .iter()
            .map(|(n, t)| (n.clone(), f32::from_bits(t.peak.swap(0, Ordering::Relaxed))))
            .collect()
    }

    pub fn clear(&mut self) {
        for tap in self.taps.values() {
            let _ = tap.stream.disconnect();
        }
        self.taps.clear();
    }
}

fn make_tap(core: &pw::core::CoreRc, bus: &str) -> Result<Tap, pw::Error> {
    let props = pw::properties::properties! {
        "media.type" => "Audio",
        "media.category" => "Capture",
        "media.role" => "DSP",
        "node.name" => format!("iara.meter.{}", bus.strip_prefix("iara.").unwrap_or(bus)).as_str(),
        "node.description" => "Iara (medidor)",
        "target.object" => bus,
        "stream.capture.sink" => "true",
        "node.passive" => "true",
        "node.dont-fallback" => "true",
        "node.dont-reconnect" => "true",
        "state.restore-props" => "false",
        "state.restore-target" => "false",
        "iara.managed" => "true",
    };
    let stream = StreamRc::new(core.clone(), "iara-meter", props)?;
    let peak = Arc::new(AtomicU32::new(0));
    let sink = peak.clone();
    let listener = stream
        .add_local_listener_with_user_data(())
        .process(move |stream, ()| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let Some(data) = buffer.datas_mut().first_mut() else {
                return;
            };
            let size = data.chunk().size() as usize;
            let offset = data.chunk().offset() as usize;
            if let Some(bytes) = data.data() {
                if let Some(block) = bytes.get(offset..offset + size) {
                    // Para f32 não negativos a ordem dos bits é a ordem numérica: `fetch_max` basta.
                    sink.fetch_max(peak_of(block).to_bits(), Ordering::Relaxed);
                }
            }
        })
        .register()?;

    let mut audio_info = spa::param::audio::AudioInfoRaw::new();
    audio_info.set_format(spa::param::audio::AudioFormat::F32LE);
    let obj = spa::pod::Object {
        type_: spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
        id: spa::param::ParamType::EnumFormat.as_raw(),
        properties: audio_info.into(),
    };
    let values = spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(obj),
    )
    .expect("serialização de pod em memória")
    .0
    .into_inner();
    let mut params = [Pod::from_bytes(&values).expect("pod válido")];
    stream.connect(
        spa::utils::Direction::Input,
        None,
        StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS | StreamFlags::RT_PROCESS,
        &mut params,
    )?;
    Ok(Tap {
        stream,
        _listener: listener,
        peak,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(samples: &[f32]) -> Vec<u8> {
        samples.iter().flat_map(|s| s.to_le_bytes()).collect()
    }

    #[test]
    fn peak_is_the_largest_absolute_sample_of_any_channel() {
        assert_eq!(peak_of(&bytes(&[0.1, -0.7, 0.3, 0.2])), 0.7);
        assert_eq!(peak_of(&bytes(&[])), 0.0);
        assert_eq!(peak_of(&bytes(&[0.0, 0.0])), 0.0);
    }

    #[test]
    fn invalid_samples_never_poison_the_peak() {
        assert_eq!(peak_of(&bytes(&[f32::NAN, 0.25, f32::INFINITY])), 0.25);
        // bloco com sobra de bytes (não múltiplo de 4) ignora o resto
        let mut b = bytes(&[0.5]);
        b.push(0xff);
        assert_eq!(peak_of(&b), 0.5);
    }

    #[test]
    fn bit_order_matches_numeric_order_for_the_atomic_max() {
        let a = AtomicU32::new(0);
        for v in [0.1f32, 0.9, 0.4, 0.0] {
            a.fetch_max(v.to_bits(), Ordering::Relaxed);
        }
        assert_eq!(f32::from_bits(a.load(Ordering::Relaxed)), 0.9);
    }
}
