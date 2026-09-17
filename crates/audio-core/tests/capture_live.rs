//! Live capture against real hardware.
//!
//! `#[ignore]` because CI has no audio interface and macOS needs microphone
//! permission, which cannot be granted non-interactively.
//!
//! ```sh
//! # list what the machine offers
//! cargo test -p audio-core --test capture_live -- --ignored --nocapture list_devices
//!
//! # record ten seconds of two channels into separate WAVs
//! CAPTURE_DEVICE="Scarlett 18i20" CAPTURE_CHANNELS=0,1 \
//!   cargo test -p audio-core --test capture_live -- --ignored --nocapture record
//! ```

use audio_core::capture::{ChannelAssignment, ChannelMap, SampleSink};
use audio_core::device::{AudioBackend, CaptureConfig, CaptureEvent, DeviceId};
use audio_core::{CpalBackend, StreamId};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Sink that keeps everything, for offline inspection. Allocating on push
/// disqualifies it from production use — the realtime path uses
/// [`audio_core::ring::RingSink`] — but the capacity is reserved up front so
/// it is good enough for a bounded recording.
struct RecordingSink {
    samples: Arc<Mutex<Vec<f32>>>,
}

impl RecordingSink {
    /// Not called `new`: it hands back a sink/handle pair rather than Self,
    /// because the demuxer takes ownership of the sink.
    fn pair(capacity: usize) -> (Box<dyn SampleSink>, Arc<Mutex<Vec<f32>>>) {
        let samples = Arc::new(Mutex::new(Vec::with_capacity(capacity)));
        (
            Box::new(Self {
                samples: samples.clone(),
            }),
            samples,
        )
    }
}

impl SampleSink for RecordingSink {
    fn push(&mut self, samples: &[f32]) -> usize {
        self.samples.lock().unwrap().extend_from_slice(samples);
        samples.len()
    }
}

#[test]
#[ignore = "needs audio hardware and microphone permission"]
fn list_devices() {
    let backend = CpalBackend::new();
    let devices = backend.input_devices().expect("enumerating devices");

    assert!(!devices.is_empty(), "no input device found at all");

    println!("\n{} input device(s):", devices.len());
    for device in &devices {
        println!(
            "  {}{}\n    channels: {}, default: {} Hz, supported: {:?}",
            device.name,
            if device.is_default { "  [default]" } else { "" },
            device.channels,
            device.default_sample_rate,
            device.sample_rates,
        );
    }

    // Every listed device must be openable by its own id — otherwise the UI
    // would offer entries that fail the moment they are picked.
    for device in &devices {
        backend
            .device(&device.id)
            .unwrap_or_else(|e| panic!("listed device '{}' not retrievable: {e}", device.name));
    }
}

#[test]
#[ignore = "needs audio hardware and microphone permission"]
fn record() {
    let backend = CpalBackend::new();

    let device = match std::env::var("CAPTURE_DEVICE") {
        Ok(name) => backend
            .device(&DeviceId(name.clone()))
            .unwrap_or_else(|e| panic!("device '{name}': {e}")),
        Err(_) => backend
            .input_devices()
            .expect("enumerating devices")
            .into_iter()
            .find(|d| d.is_default)
            .expect("no default input device"),
    };

    let channels: Vec<u16> = std::env::var("CAPTURE_CHANNELS")
        .ok()
        .map(|s| {
            s.split(',')
                .map(|c| c.trim().parse().expect("CAPTURE_CHANNELS must be numbers"))
                .collect()
        })
        .unwrap_or_else(|| (0..device.channels.min(2)).collect());

    let seconds: u64 = std::env::var("CAPTURE_SECONDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);

    println!(
        "\ndevice: {} ({} channels @ {} Hz)\nrecording channels {:?} for {seconds}s",
        device.name, device.channels, device.default_sample_rate, channels
    );

    let map = ChannelMap::new(
        channels
            .iter()
            .enumerate()
            .map(|(i, &channel)| ChannelAssignment {
                stream_id: StreamId(i as u16),
                channel,
            })
            .collect(),
        device.channels,
    )
    .expect("channel map");

    let capacity = device.default_sample_rate as usize * (seconds as usize + 1);
    let mut sinks: Vec<Box<dyn SampleSink>> = Vec::new();
    let mut recorded = Vec::new();
    for _ in &channels {
        let (sink, handle) = RecordingSink::pair(capacity);
        sinks.push(sink);
        recorded.push(handle);
    }

    let (tx, rx) = mpsc::channel();
    let session = backend
        .open(
            CaptureConfig {
                device: device.id.clone(),
                channel_map: map,
                sample_rate: None,
            },
            sinks,
            tx,
        )
        .expect("opening capture");

    let sample_rate = session.sample_rate();
    std::thread::sleep(Duration::from_secs(seconds));

    let stats_frames = session.stats().frames_captured();
    let stats_dropped = session.stats().samples_dropped();
    session.stop().expect("stopping capture");

    for event in rx.try_iter() {
        match event {
            CaptureEvent::Disconnected { device, reason } => {
                panic!("device '{device}' disconnected during the test: {reason}")
            }
            CaptureEvent::Error { message } => println!("backend error: {message}"),
        }
    }

    println!("captured {stats_frames} frames at {sample_rate} Hz, {stats_dropped} samples dropped");

    // Silence is a valid recording, but *no* frames means the microphone
    // permission was denied or the device delivered nothing — the failure this
    // test exists to catch.
    assert!(
        stats_frames > 0,
        "no frames captured — check the microphone permission"
    );
    assert_eq!(stats_dropped, 0, "sinks could not keep up");

    let expected = sample_rate as u64 * seconds;
    let tolerance = expected / 5; // generous: startup and teardown cost frames
    assert!(
        stats_frames.abs_diff(expected) < tolerance,
        "expected roughly {expected} frames, got {stats_frames}"
    );

    let out_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .join(".scratch/capture");
    std::fs::create_dir_all(&out_dir).expect("creating output directory");

    for (i, (&channel, handle)) in channels.iter().zip(recorded.iter()).enumerate() {
        let samples = handle.lock().unwrap();
        assert_eq!(
            samples.len() as u64,
            stats_frames,
            "stream {i} received a different number of samples than frames captured"
        );

        let peak = samples.iter().fold(0.0f32, |acc, s| acc.max(s.abs()));
        let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();

        let path = out_dir.join(format!("stream{i}_ch{channel}.wav"));
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut writer = hound::WavWriter::create(&path, spec).expect("creating WAV");
        for &sample in samples.iter() {
            writer.write_sample(sample).expect("writing sample");
        }
        writer.finalize().expect("finalising WAV");

        println!(
            "  {} — channel {channel}, peak {peak:.4}, rms {rms:.4}",
            path.display()
        );
    }

    println!(
        "\nListen to the files and confirm each one carries the channel it is named after.\n\
         Speaking into exactly one input at a time makes a swap obvious."
    );
}
