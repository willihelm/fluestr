//! A WAV file played in as if it were an audio interface.
//!
//! For testing settings rather than for shows: the same recording, run through
//! the same pipeline again and again, with a different model, threshold or
//! switch each time. A file is a device whose id is `file:` and the file's
//! path; it has as many channels as the file and runs at the file's rate.
//!
//! **In real time, not as fast as possible.** Everything downstream is built
//! for a live input — the rings hold four seconds, the segmenter closes on a
//! silence measured in wall-clock time, the pool sheds stale segments when it
//! falls behind. Pushing a file through faster would test the pool's overload
//! path rather than the setting under test. So the file is read in 10 ms
//! blocks against a clock, exactly as a device would deliver it.
//!
//! **A file waits; a device cannot.** A device that finds a ring full drops
//! the samples, because the sound in the room will not wait. A file can, and
//! it should: the session opens the input before its speech detectors are
//! loaded, which on a cold start takes seconds, and a recording that lost its
//! first lines to that tests nothing. So when a ring is full the file pauses
//! until there is room, and only then reads on.
//!
//! **Once, then silence.** When the file ends the "device" goes on delivering
//! silence, so the last segment is closed by the segmenter like any other and
//! the meters fall back to the floor. Stop and Start plays it again from the
//! beginning.
//!
//! WAV only — 16/24/32-bit integer or 32-bit float, any rate, any channel
//! count. That is what every DAW and recorder exports, and reading it needs
//! nothing beyond `hound`.

use crate::capture::{CaptureStats, ChannelMap, SampleSink};
use crate::device::{CaptureConfig, CaptureEvent, CaptureSession, DeviceId, DeviceInfo};
use crate::{AudioError, Result};
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// What a file's device id starts with.
pub const PREFIX: &str = "file:";

/// How much is delivered per step, like a device's buffer.
const BLOCK: Duration = Duration::from_millis(10);

/// Whether `id` names a file rather than an interface.
pub fn is_file(id: &DeviceId) -> bool {
    id.0.starts_with(PREFIX)
}

/// The device id of the file at `path`.
pub fn id_for(path: &Path) -> DeviceId {
    DeviceId(format!("{PREFIX}{}", path.display()))
}

fn path_of(id: &DeviceId) -> Result<PathBuf> {
    id.0.strip_prefix(PREFIX)
        .map(PathBuf::from)
        .ok_or_else(|| AudioError::DeviceNotFound(id.0.clone()))
}

type Reader = hound::WavReader<BufReader<std::fs::File>>;

fn open_reader(id: &DeviceId) -> Result<(Reader, PathBuf)> {
    let path = path_of(id)?;
    let reader = hound::WavReader::open(&path).map_err(|err| match err {
        hound::Error::IoError(io) if io.kind() == std::io::ErrorKind::NotFound => {
            AudioError::DeviceNotFound(format!("{} (file not found)", path.display()))
        }
        other => AudioError::Backend(format!("{}: {other}", path.display())),
    })?;
    Ok((reader, path))
}

/// The file as a device: its name, channels and rate.
pub fn device(id: &DeviceId) -> Result<DeviceInfo> {
    let (reader, path) = open_reader(id)?;
    let spec = reader.spec();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    Ok(DeviceInfo {
        id: id.clone(),
        name: format!("{name} (audio file)"),
        channels: spec.channels,
        default_sample_rate: spec.sample_rate,
        sample_rates: vec![spec.sample_rate],
        is_default: false,
    })
}

/// Starts playing the file into `sinks`.
pub fn open(
    config: CaptureConfig,
    sinks: Vec<Box<dyn SampleSink>>,
    events: Sender<CaptureEvent>,
) -> Result<Box<dyn CaptureSession>> {
    let (reader, path) = open_reader(&config.device)?;
    let spec = reader.spec();
    if let Some(rate) = config.sample_rate {
        if rate != spec.sample_rate {
            return Err(AudioError::Backend(format!(
                "{} runs at {} Hz, not {rate} Hz",
                path.display(),
                spec.sample_rate
            )));
        }
    }
    let stats = Arc::new(CaptureStats::default());
    let splitter = Splitter::new(&config.channel_map, spec.channels, sinks)?;
    let stop = Arc::new(AtomicBool::new(false));
    let seconds = reader.duration() as f32 / spec.sample_rate as f32;

    let thread_stop = stop.clone();
    let thread = std::thread::Builder::new()
        .name("audio-file".into())
        .spawn(move || play(reader, splitter, thread_stop, events))
        .map_err(|e| AudioError::Backend(format!("spawning the file reader: {e}")))?;

    tracing::info!(
        file = %path.display(),
        channels = spec.channels,
        sample_rate = spec.sample_rate,
        seconds,
        "playing an audio file as the input"
    );

    Ok(Box::new(FileSession {
        stop,
        thread: Some(thread),
        sample_rate: spec.sample_rate,
        channels: spec.channels,
        stats,
    }))
}

/// The file's channels, split into the streams' sinks — with what a full sink
/// did not take kept back for the next step rather than dropped.
struct Splitter {
    channels: Vec<u16>,
    sinks: Vec<Box<dyn SampleSink>>,
    /// Per sink: samples it has not taken yet.
    pending: Vec<Vec<f32>>,
}

impl Splitter {
    fn new(map: &ChannelMap, file_channels: u16, sinks: Vec<Box<dyn SampleSink>>) -> Result<Self> {
        if sinks.len() != map.len() {
            return Err(AudioError::Backend(format!(
                "{} sinks for {} mapped streams",
                sinks.len(),
                map.len()
            )));
        }
        if map.highest_channel() >= file_channels {
            return Err(AudioError::ChannelOutOfRange {
                channel: map.highest_channel(),
                available: file_channels,
            });
        }
        Ok(Self {
            channels: map.assignments().iter().map(|a| a.channel).collect(),
            pending: vec![Vec::new(); sinks.len()],
            sinks,
        })
    }

    /// Hands every sink what it is still owed. Whether all of them took all
    /// of it — only then may the next block be read.
    fn flush(&mut self) -> bool {
        let mut done = true;
        for (sink, pending) in self.sinks.iter_mut().zip(self.pending.iter_mut()) {
            if !pending.is_empty() {
                let taken = sink.push(pending);
                pending.drain(..taken);
                done &= pending.is_empty();
            }
        }
        done
    }

    /// Queues one interleaved block for every sink, and hands it over.
    fn split(&mut self, interleaved: &[f32], file_channels: usize) {
        for (pending, &channel) in self.pending.iter_mut().zip(self.channels.iter()) {
            pending.extend(
                interleaved
                    .chunks_exact(file_channels)
                    .map(|frame| frame[channel as usize]),
            );
        }
        self.flush();
    }
}

/// Runs on the file's own thread until stopped.
fn play(
    mut reader: Reader,
    mut splitter: Splitter,
    stop: Arc<AtomicBool>,
    events: Sender<CaptureEvent>,
) {
    let spec = reader.spec();
    let channels = spec.channels as usize;
    let frames = (spec.sample_rate as usize * BLOCK.as_millis() as usize / 1000).max(1);
    let mut block = vec![0.0f32; frames * channels];
    let mut samples = Samples::new(&mut reader);
    let mut ended = false;
    let mut next = Instant::now();

    while !stop.load(Ordering::Relaxed) {
        if !splitter.flush() {
            // A ring is full: wait for the reader on the other side, and do
            // not count the wait against the file's clock.
            std::thread::sleep(BLOCK);
            next = Instant::now();
            continue;
        }
        if !ended {
            match samples.fill(&mut block) {
                Ok(filled) if filled == block.len() => {}
                Ok(filled) => {
                    block[filled..].fill(0.0);
                    ended = true;
                    tracing::info!("the audio file has ended; delivering silence from here");
                }
                Err(err) => {
                    block.fill(0.0);
                    ended = true;
                    let _ = events.send(CaptureEvent::Error {
                        message: format!("reading the audio file: {err}"),
                    });
                }
            }
        } else {
            block.fill(0.0);
        }
        splitter.split(&block, channels);

        // Against the clock, not per step: a late wake-up is caught up on the
        // next one rather than slowing the whole file down.
        next += BLOCK;
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else if now - next > Duration::from_millis(500) {
            // The machine was asleep or stalled. Starting over from now is a
            // gap in the audio; catching up would be a burst no device makes.
            next = now;
        }
    }
}

/// The file's samples as f32 in [-1, 1], whatever they are stored as.
enum Samples<'a> {
    Float(hound::WavSamples<'a, BufReader<std::fs::File>, f32>),
    Int {
        samples: hound::WavSamples<'a, BufReader<std::fs::File>, i32>,
        scale: f32,
    },
}

impl<'a> Samples<'a> {
    fn new(reader: &'a mut Reader) -> Self {
        let spec = reader.spec();
        match spec.sample_format {
            hound::SampleFormat::Float => Samples::Float(reader.samples::<f32>()),
            hound::SampleFormat::Int => Samples::Int {
                scale: 1.0 / (1i64 << (spec.bits_per_sample - 1)) as f32,
                samples: reader.samples::<i32>(),
            },
        }
    }

    /// Fills as much of `out` as the file has left. Returns how much.
    fn fill(&mut self, out: &mut [f32]) -> std::result::Result<usize, hound::Error> {
        let mut filled = 0;
        for slot in out.iter_mut() {
            let next = match self {
                Samples::Float(samples) => samples.next(),
                Samples::Int { samples, scale } => {
                    samples.next().map(|s| s.map(|v| v as f32 * *scale))
                }
            };
            match next {
                Some(sample) => *slot = sample?,
                None => break,
            }
            filled += 1;
        }
        Ok(filled)
    }
}

struct FileSession {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    sample_rate: u32,
    channels: u16,
    stats: Arc<CaptureStats>,
}

impl CaptureSession for FileSession {
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn channels(&self) -> u16 {
        self.channels
    }

    fn stats(&self) -> &Arc<CaptureStats> {
        &self.stats
    }

    fn stop(mut self: Box<Self>) -> Result<()> {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| AudioError::Backend("the file reader panicked".into()))?;
        }
        Ok(())
    }
}

impl Drop for FileSession {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::{ChannelAssignment, ChannelMap};
    use crate::segment::StreamId;
    use std::sync::Mutex;

    #[derive(Clone, Default)]
    struct Recorded(Arc<Mutex<Vec<f32>>>);

    impl SampleSink for Recorded {
        fn push(&mut self, samples: &[f32]) -> usize {
            self.0.lock().unwrap().extend_from_slice(samples);
            samples.len()
        }
    }

    fn write_wav(path: &Path, channels: u16, frames: usize) {
        let spec = hound::WavSpec {
            channels,
            sample_rate: 8_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for frame in 0..frames {
            for channel in 0..channels {
                // Channel 0 positive, channel 1 negative, so a swap shows.
                let value = (frame as i16 % 100 + 1) * if channel == 0 { 100 } else { -100 };
                writer.write_sample(value).unwrap();
            }
        }
        writer.finalize().unwrap();
    }

    fn temp_wav(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fluestr-file-backend-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn ids_round_trip() {
        let path = Path::new("/tmp/show recording.wav");
        let id = id_for(path);
        assert!(is_file(&id));
        assert_eq!(path_of(&id).unwrap(), path);
        assert!(!is_file(&DeviceId("BuiltInMicrophoneDevice".into())));
    }

    #[test]
    fn the_file_is_described_as_a_device() {
        let path = temp_wav("describe.wav");
        write_wav(&path, 2, 800);
        let info = device(&id_for(&path)).unwrap();
        assert_eq!(info.channels, 2);
        assert_eq!(info.default_sample_rate, 8_000);
        assert_eq!(info.name, "describe.wav (audio file)");
    }

    #[test]
    fn a_missing_file_is_a_device_not_found() {
        let err = device(&id_for(Path::new("/nonexistent/x.wav"))).unwrap_err();
        assert!(matches!(err, AudioError::DeviceNotFound(_)), "{err}");
    }

    /// Takes nothing for its first `refuse` pushes, then everything.
    #[derive(Clone)]
    struct Slow {
        recorded: Recorded,
        refuse: Arc<Mutex<usize>>,
    }

    impl SampleSink for Slow {
        fn push(&mut self, samples: &[f32]) -> usize {
            let mut refuse = self.refuse.lock().unwrap();
            if *refuse > 0 {
                *refuse -= 1;
                return 0;
            }
            self.recorded.push(samples)
        }
    }

    #[test]
    fn a_full_sink_makes_the_file_wait_rather_than_lose_audio() {
        let path = temp_wav("wait.wav");
        write_wav(&path, 1, 800);
        let map = ChannelMap::new(
            vec![ChannelAssignment {
                stream_id: StreamId(0),
                channel: 0,
            }],
            1,
        )
        .unwrap();
        let sink = Slow {
            recorded: Recorded::default(),
            refuse: Arc::new(Mutex::new(5)),
        };
        let (events, _rx) = std::sync::mpsc::channel();
        let session = open(
            CaptureConfig {
                device: id_for(&path),
                channel_map: map,
                sample_rate: Some(8_000),
            },
            vec![Box::new(sink.clone())],
            events,
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(300));
        session.stop().unwrap();

        // Every one of the file's samples, in order, despite the refusals.
        let got = sink.recorded.0.lock().unwrap().clone();
        assert!(got.len() >= 800, "only {} samples arrived", got.len());
        for (frame, &sample) in got[..800].iter().enumerate() {
            let expected = (frame as i16 % 100 + 1) as f32 * 100.0 / 32768.0;
            assert!((sample - expected).abs() < 1e-6, "frame {frame}: {sample}");
        }
    }

    #[test]
    fn the_second_channel_arrives_in_real_time_then_silence() {
        let path = temp_wav("play.wav");
        // 100 ms at 8 kHz.
        write_wav(&path, 2, 800);
        let id = id_for(&path);
        let map = ChannelMap::new(
            vec![ChannelAssignment {
                stream_id: StreamId(0),
                channel: 1,
            }],
            2,
        )
        .unwrap();
        let recorded = Recorded::default();
        let (events, _rx) = std::sync::mpsc::channel();
        let started = Instant::now();
        let session = open(
            CaptureConfig {
                device: id,
                channel_map: map,
                sample_rate: Some(8_000),
            },
            vec![Box::new(recorded.clone())],
            events,
        )
        .unwrap();

        std::thread::sleep(Duration::from_millis(250));
        session.stop().unwrap();
        let got = recorded.0.lock().unwrap().clone();

        // The file's 800 frames, from channel 1 only (negative), in order…
        assert!(got.len() >= 800, "only {} samples arrived", got.len());
        assert!(got[..800].iter().all(|&s| s < 0.0));
        assert!((got[0] - (-100.0 / 32768.0)).abs() < 1e-6);
        // …then silence, and not faster than the clock: 250 ms of 8 kHz is
        // 2000 samples, give or take a block.
        assert!(got[800..].iter().all(|&s| s == 0.0));
        let elapsed = started.elapsed().as_secs_f32();
        assert!(
            (got.len() as f32) <= elapsed * 8_000.0 + 160.0,
            "{} samples in {elapsed} s is faster than real time",
            got.len()
        );
    }
}
