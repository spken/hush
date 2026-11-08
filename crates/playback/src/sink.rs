use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use arc_swap::ArcSwap;
use cpal::{
    BufferSize, DeviceId, Stream, StreamConfig,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use librespot_playback::{
    NUM_CHANNELS, SAMPLE_RATE,
    audio_backend::{Sink, SinkError, SinkResult},
    convert::Converter,
    decoder::AudioPacket,
    mixer::VolumeGetter,
};
use rtrb::{Consumer, Producer, RingBuffer};

use crate::eq::{Coefficients, Filters};

/// The output device's ID, or `None` for the system default; shared with `Device`.
pub(crate) type OutputChoice = Arc<ArcSwap<Option<Box<str>>>>;

const BUFFER_SAMPLES: usize = SAMPLE_RATE as usize / 10 * NUM_CHANNELS as usize;
const FULL_WAIT: Duration = Duration::from_millis(5);
const STALL_LIMIT: Duration = Duration::from_secs(1);
// One-pole smoothing with a ~10 ms time constant, so volume steps do not click.
const VOLUME_SMOOTHING: f64 = 1.0 / (0.01 * SAMPLE_RATE as f64);
// The limiter starts bending the signal at -1 dBFS and never reaches 0 dBFS.
const LIMITER_KNEE: f64 = 0.891;

/// librespot's audio sink for Hush: preamp and EQ, then volume, then a soft-knee limiter,
/// into a ~100 ms ring buffer that a WASAPI shared-mode stream drains. Windows converts
/// the 44.1 kHz stream to the device's rate.
pub(crate) struct EqSink {
    filters: Filters,
    volume: Box<dyn VolumeGetter + Send>,
    gain: f64,
    choice: OutputChoice,
    // Opened on the first `start`, because building a sink cannot fail, and again when the
    // choice of device changes.
    output: Option<Output>,
}

struct Output {
    stream: Stream,
    producer: Producer<f32>,
    /// The choice it was opened for.
    choice: Arc<Option<Box<str>>>,
}

impl EqSink {
    pub(crate) fn new(
        coefficients: Arc<ArcSwap<Coefficients>>,
        volume: Box<dyn VolumeGetter + Send>,
        choice: OutputChoice,
    ) -> Self {
        let gain = volume.attenuation_factor();
        Self {
            filters: Filters::new(coefficients),
            volume,
            gain,
            choice,
            output: None,
        }
    }

    /// The open output, opened afresh when there is none or another device was chosen.
    fn output(&mut self) -> SinkResult<&mut Output> {
        let choice = self.choice.load_full();
        let current =
            (self.output.as_ref()).is_some_and(|output| Arc::ptr_eq(&output.choice, &choice));
        if !current {
            // The old stream stops when dropped, before the new one opens.
            self.output = None;
            let output = Output::open(choice)?;
            output
                .stream
                .play()
                .map_err(|e| SinkError::StateChange(e.to_string()))?;
            self.output = Some(output);
        }
        self.output
            .as_mut()
            .ok_or_else(|| SinkError::NotConnected("no output".into()))
    }
}

impl Sink for EqSink {
    fn start(&mut self) -> SinkResult<()> {
        self.output()?
            .stream
            .play()
            .map_err(|e| SinkError::StateChange(e.to_string()))
    }

    fn stop(&mut self) -> SinkResult<()> {
        if let Some(output) = &self.output {
            output
                .stream
                .pause()
                .map_err(|e| SinkError::StateChange(e.to_string()))?;
        }
        Ok(())
    }

    fn write(&mut self, packet: AudioPacket, _: &mut Converter) -> SinkResult<()> {
        let AudioPacket::Samples(mut samples) = packet else {
            return Err(SinkError::InvalidParams("passthrough is off".into()));
        };
        self.filters.update();
        let volume = self.volume.attenuation_factor();
        for frame in samples.chunks_exact_mut(NUM_CHANNELS.into()) {
            self.gain += (volume - self.gain) * VOLUME_SMOOTHING;
            self.filters.process(frame);
            for sample in frame {
                *sample = limit(*sample * self.gain);
            }
        }
        self.output()?.push(&samples)
    }
}

impl Output {
    /// Opens the device `choice` names, or the system default when it names none or one
    /// that is gone.
    fn open(choice: Arc<Option<Box<str>>>) -> SinkResult<Self> {
        let refused = |e: &dyn std::fmt::Display| SinkError::ConnectionRefused(e.to_string());
        let host = cpal::default_host();
        let chosen = (choice.as_deref())
            .and_then(|id| id.parse::<DeviceId>().ok())
            .and_then(|id| host.device_by_id(&id));
        if choice.is_some() && chosen.is_none() {
            tracing::warn!("chosen output device is gone; using the default");
        }
        let device = chosen
            .or_else(|| host.default_output_device())
            .ok_or_else(|| refused(&"no default output device"))?;
        let config = StreamConfig {
            channels: NUM_CHANNELS.into(),
            sample_rate: SAMPLE_RATE,
            buffer_size: BufferSize::Default,
        };
        let (producer, mut consumer) = RingBuffer::new(BUFFER_SAMPLES);
        let stream = device
            .build_output_stream(
                config,
                move |data: &mut [f32], _| drain(&mut consumer, data),
                |error| tracing::warn!(%error, "audio output"),
                None,
            )
            .map_err(|e| refused(&e))?;
        Ok(Self {
            stream,
            producer,
            choice,
        })
    }

    /// Queues `samples`, waiting while the buffer is full; that wait is what paces the
    /// player to real time.
    fn push(&mut self, samples: &[f64]) -> SinkResult<()> {
        let mut rest = samples;
        let mut stalled_since = Instant::now();
        while !rest.is_empty() {
            // Whole frames only, so the stream never swaps channels.
            let n = self.producer.slots().min(rest.len()) & !1;
            if n == 0 {
                if stalled_since.elapsed() > STALL_LIMIT {
                    return Err(SinkError::OnWrite("audio device stopped playing".into()));
                }
                thread::sleep(FULL_WAIT);
                continue;
            }
            let chunk = self
                .producer
                .write_chunk_uninit(n)
                .map_err(|e| SinkError::OnWrite(e.to_string()))?;
            #[allow(clippy::cast_possible_truncation)]
            chunk.fill_from_iter(rest[..n].iter().map(|&sample| sample as f32));
            rest = &rest[n..];
            stalled_since = Instant::now();
        }
        Ok(())
    }
}

/// Fills one device buffer from the ring buffer, with silence for whatever is missing.
/// Runs on the real-time audio thread: no allocation, locks or logging.
fn drain(consumer: &mut Consumer<f32>, data: &mut [f32]) {
    let n = consumer.slots().min(data.len()) & !1;
    let Ok(chunk) = consumer.read_chunk(n) else {
        data.fill(0.0);
        return;
    };
    let (first, second) = chunk.as_slices();
    let (head, tail) = data.split_at_mut(n);
    head[..first.len()].copy_from_slice(first);
    head[first.len()..].copy_from_slice(second);
    chunk.commit_all();
    tail.fill(0.0);
}

/// Soft-knee limiter: passes the signal untouched below the knee and bends it smoothly
/// toward, but never past, full scale above it.
fn limit(sample: f64) -> f64 {
    let magnitude = sample.abs();
    if magnitude <= LIMITER_KNEE {
        return sample;
    }
    let headroom = 1.0 - LIMITER_KNEE;
    (LIMITER_KNEE + headroom * ((magnitude - LIMITER_KNEE) / headroom).tanh()).copysign(sample)
}

/// The audio outputs Windows offers. Blocks while it asks.
pub fn output_devices() -> Vec<model::OutputDevice> {
    let devices = match cpal::default_host().output_devices() {
        Ok(devices) => devices,
        Err(error) => {
            tracing::warn!(%error, "output devices not listed");
            return Vec::new();
        }
    };
    devices
        .filter_map(|device| {
            Some(model::OutputDevice {
                id: device.id().ok()?.to_string().into(),
                name: device.description().ok()?.name().into(),
            })
        })
        .collect()
}
