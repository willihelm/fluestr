//! Where audio leaves the app (P22, P26).
//!
//! Two outputs, and deliberately two:
//!
//! - the **listen** output, where a bubble's audio is replayed — headphones,
//!   typically, so a replay reaches the operator and nobody else;
//! - the **shoutbox**: one intercom channel passed through to a talkback
//!   speaker, its level following the room (see [`audio_core::shoutbox`]).
//!
//! **This is the app in the signal path.** Until these existed it only
//! listened; if the shoutbox stops, a position goes deaf, and because nobody
//! may be talking at that moment nobody notices. Hence the shape of this
//! module:
//!
//! - it is **independent of the transcription session**. Stopping a session,
//!   changing the model, a sidecar restart — none of it reaches here. The
//!   shoutbox opens its own streams (CoreAudio and WASAPI shared mode are both
//!   multi-client) and runs whenever it is switched on and its devices are
//!   there;
//! - it **follows the configuration file**, like the LAN remote (P16): the
//!   active profile's outputs are applied at startup and on every save, and
//!   there is no second "start the shoutbox" command to disagree with it;
//! - a device that is missing is **retried**, never replaced. Falling back to
//!   another output could put talkback on the PA.

use crate::config::{InputRoute, OutputRoute, Outputs, Shoutbox};
use crate::replay::{ClipKey, ReplayStore};
use audio_core::shoutbox::{self, Controller, ShoutboxSource, WeightingFilter};
use audio_core::{
    playback, ring, AudioBackend, CaptureConfig, CaptureEvent, CaptureSession, ChannelAssignment,
    ChannelMap, CpalBackend, DeviceId, OutputConfig, PlaybackSession, PlayerHandle, RingSource,
    StreamId, SAMPLE_RATE,
};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// How often the controller runs.
const TICK: Duration = Duration::from_millis(20);

/// Status events per second — enough for a meter, few enough not to matter
/// on the bridge.
const STATUS_EVERY: Duration = Duration::from_millis(100);

/// How long a shoutbox whose device is missing waits before trying again.
const RETRY: Duration = Duration::from_secs(3);

/// How much program audio the pass-through ring holds. The source keeps its
/// own fill far below this; the rest is headroom for a scheduling hiccup.
const PROGRAM_RING_SECONDS: f32 = 0.5;

/// Where a status update goes. A closure rather than the app handle, so this
/// module stays free of Tauri like the session is.
pub type Emit = Arc<dyn Fn(ShoutboxStatus) + Send + Sync>;

/// What the shoutbox is doing, for the meter beside its controls.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "outputs.ts")]
pub enum ShoutboxState {
    /// Switched off in the profile.
    #[default]
    Off,
    /// Opening its devices.
    Starting,
    Running,
    /// Could not open, or lost, a device. Retried every few seconds.
    Failed,
}

/// The shoutbox as its controls show it (P26).
#[derive(Debug, Clone, Default, PartialEq, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "outputs.ts")]
pub struct ShoutboxStatus {
    pub state: ShoutboxState,
    /// Why it is not running, in words an operator can act on.
    pub message: Option<String>,
    /// The loudest pre-gain sample of the program since the last status, in
    /// dBFS. `None` for silence.
    pub program_dbfs: Option<f32>,
    /// The averaged room level, in dB SPL when calibrated and weighted dBFS
    /// when not. Measured in the gaps only: what the gain follows.
    pub level_db: Option<f32>,
    /// The room now, gate or not, in the same units — the meter.
    pub live_db: Option<f32>,
    pub calibrated: bool,
    /// What the curve asks for at that level.
    pub target_db: f32,
    /// What is in force, before the trim.
    pub gain_db: f32,
    /// Whether the room is being measured: false while the shoutbox plays.
    pub measuring: bool,
    /// False when the reference reads as a missing mic.
    pub reference_ok: bool,
    /// How deep the limiter is working, in dB (0 = not at all).
    pub limiting_db: f32,
    /// Callbacks that ran dry, and samples skipped to bound the latency —
    /// both zero on one clock, and the first sign of two.
    pub underruns: u64,
    pub skipped: u64,
}

impl ShoutboxStatus {
    fn off() -> Self {
        Self::default()
    }

    fn with(state: ShoutboxState, message: Option<String>) -> Self {
        Self {
            state,
            message,
            reference_ok: true,
            ..Self::default()
        }
    }
}

/// The answer to a replay (P22).
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "outputs.ts")]
pub struct ReplayStarted {
    /// How long it plays, so the button can show it is playing without a
    /// second event.
    pub duration_ms: u64,
    /// Where it is playing, in words — "Headphones (MADIface 7–8)".
    pub target: String,
}

/// A bubble's audio as a WAV, for a browser to play itself (P16, P22).
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "outputs.ts")]
pub struct ReplayWav {
    /// `audio/wav`, base64.
    pub wav: String,
    pub duration_ms: u64,
}

/// One output device and its channels, for the pickers (P22, P26).
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "outputs.ts")]
pub struct OutputDeviceOption {
    pub id: String,
    pub name: String,
    pub channels: u16,
    pub default_sample_rate: u32,
    pub is_default: bool,
}

pub fn list_output_devices() -> anyhow::Result<Vec<OutputDeviceOption>> {
    Ok(CpalBackend::new()
        .output_devices()?
        .into_iter()
        .map(|info| OutputDeviceOption {
            id: info.id.0,
            name: info.name,
            channels: info.channels,
            default_sample_rate: info.default_sample_rate,
            is_default: info.is_default,
        })
        .collect())
}

/// The listen output while it is open.
struct Listen {
    /// What it was opened for, so a changed route reopens it.
    route: OutputRoute,
    session: Box<dyn PlaybackSession>,
    player: PlayerHandle,
    rate: u32,
    events: mpsc::Receiver<CaptureEvent>,
    name: String,
}

/// Everything that sends audio out. One per app, in the app state.
#[derive(Default)]
pub struct Engine {
    /// What was applied last, so an unchanged save costs nothing.
    applied: Mutex<Option<Outputs>>,
    listen: Mutex<Option<Listen>>,
    shoutbox: Mutex<Option<Runner>>,
    /// The last status, for a view that opens after it was sent.
    status: Arc<Mutex<ShoutboxStatus>>,
    cues: AtomicU64,
}

impl Engine {
    /// Brings the outputs into line with the active profile's. Idempotent.
    ///
    /// Only what changed is touched: a moved curve or trim reaches the
    /// running controller without the streams being reopened — which would
    /// be an audible gap on the shoutbox, at the moment somebody is adjusting
    /// it by ear.
    pub fn apply(
        &self,
        outputs: &Outputs,
        replay: &ReplayStore,
        emit: Emit,
    ) -> Option<crate::replay::Horizon> {
        let Ok(mut applied) = self.applied.lock() else {
            return None;
        };
        if applied.as_ref() == Some(outputs) {
            return None;
        }
        let previous = applied.replace(outputs.clone());

        let horizon = replay.set_keep_minutes(outputs.listen.keep_minutes);

        // The listen output is opened lazily, at the first replay, and
        // closed here when its route changes — see `ensure_listen`.
        if let Ok(mut listen) = self.listen.lock() {
            let stale = listen
                .as_ref()
                .is_some_and(|open| open.route != outputs.listen.output);
            if stale {
                if let Some(open) = listen.take() {
                    let _ = open.session.stop();
                }
            }
        }

        if let Ok(mut runner) = self.shoutbox.lock() {
            let keep = runner
                .as_ref()
                .is_some_and(|r| r.config.same_streams(&outputs.shoutbox));
            if keep {
                if let Some(r) = runner.as_mut() {
                    r.update(outputs.shoutbox.clone());
                }
            } else {
                if let Some(old) = runner.take() {
                    old.stop();
                }
                if outputs.shoutbox.enabled {
                    *runner = Some(Runner::start(
                        outputs.shoutbox.clone(),
                        self.status.clone(),
                        emit.clone(),
                    ));
                } else if previous.is_some_and(|p| p.shoutbox.enabled)
                    || self.current_status().state != ShoutboxState::Off
                {
                    let status = ShoutboxStatus::off();
                    if let Ok(mut last) = self.status.lock() {
                        *last = status.clone();
                    }
                    emit(status);
                }
            }
        }
        horizon
    }

    /// The trim and mute, at once rather than after the save's debounce: a
    /// mute that arrives a second late is a second of talkback in the house.
    pub fn set_live(&self, trim_db: i32, muted: bool) {
        let clamp = |db: i32| db.clamp(*Shoutbox::TRIM_DB.start(), *Shoutbox::TRIM_DB.end());
        if let Ok(mut runner) = self.shoutbox.lock() {
            if let Some(r) = runner.as_mut() {
                let mut config = r.config.clone();
                config.trim_db = clamp(trim_db);
                config.muted = muted;
                r.update(config);
            }
        }
        // So the next save of the same numbers is recognised as nothing new.
        if let Ok(mut applied) = self.applied.lock() {
            if let Some(outputs) = applied.as_mut() {
                outputs.shoutbox.trim_db = clamp(trim_db);
                outputs.shoutbox.muted = muted;
            }
        }
    }

    pub fn current_status(&self) -> ShoutboxStatus {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Plays a bubble on the listen output. To have the whole position hear
    /// it, the listen output is patched to the shoutbox's speaker.
    pub fn play(&self, replay: &ReplayStore, keys: &[ClipKey]) -> Result<ReplayStarted, String> {
        let audio = replay
            .assemble(keys)
            .ok_or_else(|| "This audio is no longer held.".to_string())?;
        let duration_ms = (audio.len() as u64 * 1000) / SAMPLE_RATE as u64;
        let cue = self.cues.fetch_add(1, Ordering::Relaxed) + 1;

        let route = self
            .applied
            .lock()
            .ok()
            .and_then(|a| a.as_ref().map(|o| o.listen.output.clone()))
            .unwrap_or_default();
        let mut listen = self
            .listen
            .lock()
            .map_err(|_| "the listen output is poisoned")?;
        ensure_listen(&mut listen, &route)?;
        let open = listen.as_mut().ok_or("the listen output did not open")?;
        let samples = audio_core::resample::convert(&audio, SAMPLE_RATE, open.rate)
            .map_err(|e| e.to_string())?;
        open.player.play(cue, samples).map_err(|e| e.to_string())?;
        Ok(ReplayStarted {
            duration_ms,
            target: open.name.clone(),
        })
    }

    pub fn stop_replay(&self) {
        if let Ok(mut listen) = self.listen.lock() {
            if let Some(open) = listen.as_mut() {
                open.player.stop();
            }
        }
    }

    /// Stops everything, for the app going away.
    pub fn shutdown(&self) {
        if let Ok(mut runner) = self.shoutbox.lock() {
            if let Some(r) = runner.take() {
                r.stop();
            }
        }
        if let Ok(mut listen) = self.listen.lock() {
            if let Some(open) = listen.take() {
                let _ = open.session.stop();
            }
        }
    }
}

/// Opens the listen output if it is not open, or was lost.
///
/// Opened at the first replay rather than at startup: an output stream held
/// open all evening on a device the operator never replays to is a stream
/// that can only go wrong. A route that is missing is said so; the system
/// output is used only when that is what the route *is*.
fn ensure_listen(listen: &mut Option<Listen>, route: &OutputRoute) -> Result<(), String> {
    let lost = listen.as_ref().is_some_and(|open| {
        open.route != *route
            || open
                .events
                .try_iter()
                .any(|e| matches!(e, CaptureEvent::Disconnected { .. }))
    });
    if lost {
        if let Some(open) = listen.take() {
            let _ = open.session.stop();
        }
    }
    if listen.is_some() {
        return Ok(());
    }

    let backend = CpalBackend::new();
    let wanted = (!route.device_id.is_empty()).then(|| DeviceId(route.device_id.clone()));
    let info = backend.output_device(wanted.as_ref()).map_err(|_| {
        if route.device.is_empty() {
            "There is no system output to play on.".to_string()
        } else {
            format!("'{}' is not connected.", route.device)
        }
    })?;
    let channels = channels_or_default(&route.channels, info.channels);
    let (player, handle) = playback::player();
    let (events, events_rx) = mpsc::channel();
    let session = backend
        .open_output(
            OutputConfig {
                device: info.id.clone(),
                channels: channels.clone(),
                sample_rate: None,
            },
            Box::new(player),
            events,
        )
        .map_err(|e| format!("'{}' could not be opened: {e}", info.name))?;
    let name = format!("{} {}", info.name, channel_names(&channels));
    tracing::info!(output = %name, rate = session.sample_rate(), "listen output open");
    *listen = Some(Listen {
        route: route.clone(),
        rate: session.sample_rate(),
        session,
        player: handle,
        events: events_rx,
        name,
    });
    Ok(())
}

/// A route's channels, or the device's first pair (or only channel) when it
/// names none.
fn channels_or_default(channels: &[u16], available: u16) -> Vec<u16> {
    if !channels.is_empty() {
        return channels.to_vec();
    }
    if available >= 2 {
        vec![0, 1]
    } else {
        vec![0]
    }
}

/// "1–2", "5" — one-based, the way a patch bay is labelled.
fn channel_names(channels: &[u16]) -> String {
    match channels {
        [one] => format!("{}", one + 1),
        [a, b] if b == &(a + 1) => format!("{}–{}", a + 1, b + 1),
        many => many
            .iter()
            .map(|c| (c + 1).to_string())
            .collect::<Vec<_>>()
            .join("+"),
    }
}

/// The shoutbox's supervising thread and what the engine holds of it.
struct Runner {
    /// What it was started with, updated as numbers change.
    config: Shoutbox,
    updates: mpsc::Sender<Shoutbox>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Runner {
    fn start(config: Shoutbox, last: Arc<Mutex<ShoutboxStatus>>, emit: Emit) -> Self {
        let (updates, updates_rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (config, stop) = (config.clone(), stop.clone());
            let report: Emit = Arc::new(move |status: ShoutboxStatus| {
                if let Ok(mut l) = last.lock() {
                    *l = status.clone();
                }
                emit(status);
            });
            std::thread::Builder::new()
                .name("shoutbox".into())
                .spawn(move || supervise(config, updates_rx, stop, report))
                .ok()
        };
        Self {
            config,
            updates,
            stop,
            thread,
        }
    }

    fn update(&mut self, config: Shoutbox) {
        self.config = config.clone();
        let _ = self.updates.send(config);
    }

    fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Opens, runs and — when a device goes — reopens the shoutbox, until told
/// to stop.
fn supervise(
    mut config: Shoutbox,
    updates: mpsc::Receiver<Shoutbox>,
    stop: Arc<AtomicBool>,
    emit: Emit,
) {
    let mut controller = Controller::new(config.control());
    while !stop.load(Ordering::Relaxed) {
        emit(ShoutboxStatus::with(ShoutboxState::Starting, None));
        let failure = match open(&config) {
            Ok(mut streams) => {
                tracing::info!(rate = streams.rate, "shoutbox running");
                let failure = run(
                    &mut config,
                    &mut controller,
                    &mut streams,
                    &updates,
                    &stop,
                    &emit,
                );
                streams.close();
                failure
            }
            Err(message) => Some(message),
        };
        let Some(message) = failure else {
            break;
        };
        tracing::warn!(message, "shoutbox not running, retrying");
        emit(ShoutboxStatus::with(ShoutboxState::Failed, Some(message)));

        let until = Instant::now() + RETRY;
        while Instant::now() < until && !stop.load(Ordering::Relaxed) {
            for next in updates.try_iter() {
                controller.set_settings(next.control());
                config = next;
            }
            std::thread::sleep(TICK);
        }
    }
    emit(ShoutboxStatus::off());
}

/// The shoutbox's open streams.
struct Streams {
    captures: Vec<Box<dyn CaptureSession>>,
    playback: Box<dyn PlaybackSession>,
    reference: RingSource,
    reference_rate: u32,
    shared: Arc<shoutbox::Shared>,
    events: mpsc::Receiver<CaptureEvent>,
    rate: u32,
}

impl Streams {
    /// Output first: the speaker goes quiet before its source goes away,
    /// rather than playing the tail of a ring that is no longer filled.
    fn close(self) {
        let _ = self.playback.stop();
        for capture in self.captures {
            let _ = capture.stop();
        }
    }
}

fn patched(route: &InputRoute, what: &str) -> Result<DeviceId, String> {
    if route.device_id.is_empty() && route.device.is_empty() {
        return Err(format!("The {what} is not patched."));
    }
    Ok(DeviceId(if route.device_id.is_empty() {
        route.device.clone()
    } else {
        route.device_id.clone()
    }))
}

/// Opens the program and reference captures and the shoutbox output.
fn open(config: &Shoutbox) -> Result<Streams, String> {
    let backend = CpalBackend::new();
    let named = |route: &InputRoute| {
        if route.device.is_empty() {
            route.device_id.clone()
        } else {
            route.device.clone()
        }
    };

    let program_id = patched(&config.program, "program input")?;
    let reference_id = patched(&config.reference, "measurement microphone")?;
    if config.output.device_id.is_empty() && config.output.device.is_empty() {
        return Err("The shoutbox output is not patched.".into());
    }
    let output_id = DeviceId(if config.output.device_id.is_empty() {
        config.output.device.clone()
    } else {
        config.output.device_id.clone()
    });

    let program = backend
        .device(&program_id)
        .map_err(|_| format!("'{}' (program) is not connected.", named(&config.program)))?;
    let reference = backend.device(&reference_id).map_err(|_| {
        format!(
            "'{}' (measurement mic) is not connected.",
            named(&config.reference)
        )
    })?;
    let output = backend.output_device(Some(&output_id)).map_err(|_| {
        format!(
            "'{}' (shoutbox output) is not connected.",
            config.output.device
        )
    })?;

    // One clock for what is passed through: an interface's input and output
    // sides are one device, and so is a macOS aggregate. Two devices drift,
    // and a pass-through between them runs dry or overflows over minutes.
    if program.id != output.id && program.name != output.name {
        return Err(format!(
            "The shoutbox output has to be on the program's interface ('{}'), not on '{}': \
             two devices are two clocks.",
            program.name, output.name
        ));
    }
    if config.program.channel >= program.channels {
        return Err(format!(
            "'{}' has no input {} for the program.",
            program.name,
            config.program.channel + 1
        ));
    }
    if config.reference.channel >= reference.channels {
        return Err(format!(
            "'{}' has no input {} for the measurement mic.",
            reference.name,
            config.reference.channel + 1
        ));
    }

    let rate = program.default_sample_rate;
    let (events, events_rx) = mpsc::channel();
    let (program_sink, program_source) = ring::channel(rate, PROGRAM_RING_SECONDS);
    let mut captures: Vec<Box<dyn CaptureSession>> = Vec::new();

    let (reference_source, reference_rate) = if reference.id == program.id {
        // One capture for both, which is also one clock for both.
        let (sink, source) = ring::channel(rate, 2.0);
        let map = ChannelMap::new(
            vec![
                ChannelAssignment {
                    stream_id: StreamId(0),
                    channel: config.program.channel,
                },
                ChannelAssignment {
                    stream_id: StreamId(1),
                    channel: config.reference.channel,
                },
            ],
            program.channels,
        )
        .map_err(|e| e.to_string())?;
        captures.push(
            backend
                .open(
                    CaptureConfig {
                        device: program.id.clone(),
                        channel_map: map,
                        sample_rate: Some(rate),
                    },
                    vec![Box::new(program_sink), Box::new(sink)],
                    events.clone(),
                )
                .map_err(|e| format!("'{}' could not be opened: {e}", program.name))?,
        );
        (source, rate)
    } else {
        let map = ChannelMap::new(
            vec![ChannelAssignment {
                stream_id: StreamId(0),
                channel: config.program.channel,
            }],
            program.channels,
        )
        .map_err(|e| e.to_string())?;
        captures.push(
            backend
                .open(
                    CaptureConfig {
                        device: program.id.clone(),
                        channel_map: map,
                        sample_rate: Some(rate),
                    },
                    vec![Box::new(program_sink)],
                    events.clone(),
                )
                .map_err(|e| format!("'{}' could not be opened: {e}", program.name))?,
        );
        // Only measured, never played: its clock does not matter.
        let reference_rate = reference.default_sample_rate;
        let (sink, source) = ring::channel(reference_rate, 2.0);
        let map = ChannelMap::new(
            vec![ChannelAssignment {
                stream_id: StreamId(0),
                channel: config.reference.channel,
            }],
            reference.channels,
        )
        .map_err(|e| e.to_string())?;
        captures.push(
            backend
                .open(
                    CaptureConfig {
                        device: reference.id.clone(),
                        channel_map: map,
                        sample_rate: Some(reference_rate),
                    },
                    vec![Box::new(sink)],
                    events.clone(),
                )
                .map_err(|e| format!("'{}' could not be opened: {e}", reference.name))?,
        );
        (source, reference_rate)
    };

    let shared = Arc::new(shoutbox::Shared::default());
    shared.set_ceiling_dbfs(config.ceiling_dbfs as f32);
    let source = ShoutboxSource::new(program_source, shared.clone(), rate);
    let channels = channels_or_default(&config.output.channels, output.channels);
    let playback = backend
        .open_output(
            OutputConfig {
                device: output.id.clone(),
                channels,
                sample_rate: Some(rate),
            },
            Box::new(source),
            events,
        )
        .map_err(|e| format!("'{}' could not be opened for output: {e}", output.name))?;
    if playback.sample_rate() != rate {
        return Err(format!(
            "'{}' runs its output at {} Hz and its input at {rate} Hz.",
            output.name,
            playback.sample_rate()
        ));
    }

    Ok(Streams {
        captures,
        playback,
        reference: reference_source,
        reference_rate,
        shared,
        events: events_rx,
        rate,
    })
}

/// The control loop. Returns why it ended, or `None` for a stop.
fn run(
    config: &mut Shoutbox,
    controller: &mut Controller,
    streams: &mut Streams,
    updates: &mpsc::Receiver<Shoutbox>,
    stop: &AtomicBool,
    emit: &Emit,
) -> Option<String> {
    // Always A, like the SPL meter the calibration is read off.
    let mut filter = WeightingFilter::new(streams.reference_rate);
    let mut scratch = vec![0.0f32; streams.reference_rate as usize / 10];
    let mut last = Instant::now();
    let mut last_status = Instant::now() - STATUS_EVERY;
    let mut program_peak = 0.0f32;
    let mut limiting = 0.0f32;

    while !stop.load(Ordering::Relaxed) {
        for next in updates.try_iter() {
            controller.set_settings(next.control());
            streams.shared.set_ceiling_dbfs(next.ceiling_dbfs as f32);
            *config = next;
        }
        for event in streams.events.try_iter() {
            match event {
                CaptureEvent::Disconnected { reason, .. } => {
                    return Some(format!("A shoutbox device went away: {reason}."));
                }
                CaptureEvent::Error { message } => tracing::warn!(message, "shoutbox stream error"),
            }
        }

        // Everything the mic delivered since the last step, weighted.
        let mut sum = 0.0f64;
        let mut count = 0usize;
        loop {
            let read = streams.reference.read(&mut scratch);
            if read == 0 {
                break;
            }
            if let Some(ms) = filter.mean_square(&scratch[..read]) {
                sum += ms * read as f64;
                count += read;
            }
        }
        let reference = (count > 0).then(|| sum / count as f64);

        let peak = streams.shared.take_peak();
        program_peak = program_peak.max(peak);
        let now = Instant::now();
        let reading = controller.step(reference, peak, now - last);
        last = now;

        let gain = if config.muted {
            0.0
        } else {
            shoutbox::linear(reading.gain_db + config.trim_db as f32)
        };
        streams.shared.set_gain(gain);
        limiting = limiting.min(streams.shared.take_limiting_db());

        if now - last_status >= STATUS_EVERY {
            emit(ShoutboxStatus {
                state: ShoutboxState::Running,
                message: (!reading.reference_ok).then(|| {
                    "The measurement mic reads as disconnected — the room level is held.".into()
                }),
                program_dbfs: (program_peak > 0.0).then(|| 20.0 * program_peak.log10()),
                level_db: reading.level_db.filter(|l| l.is_finite()),
                live_db: reading.live_db.filter(|l| l.is_finite()),
                calibrated: config.calibration_db.is_some(),
                target_db: reading.target_db,
                gain_db: reading.gain_db,
                measuring: reading.measuring,
                reference_ok: reading.reference_ok,
                limiting_db: limiting,
                underruns: streams.shared.underruns(),
                skipped: streams.shared.skipped(),
            });
            last_status = now;
            program_peak = 0.0;
            limiting = 0.0;
        }

        std::thread::sleep(TICK);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_route_without_channels_takes_the_first_pair() {
        assert_eq!(channels_or_default(&[], 8), vec![0, 1]);
        assert_eq!(channels_or_default(&[], 1), vec![0]);
        assert_eq!(channels_or_default(&[5], 8), vec![5]);
    }

    #[test]
    fn channels_are_named_the_way_a_patch_bay_is() {
        assert_eq!(channel_names(&[6, 7]), "7–8");
        assert_eq!(channel_names(&[4]), "5");
        assert_eq!(channel_names(&[0, 3]), "1+4");
    }

    #[test]
    fn an_unpatched_shoutbox_says_which_half_is_missing() {
        let config = Shoutbox {
            enabled: true,
            ..Shoutbox::default()
        };
        let Err(message) = open(&config) else {
            panic!("opened with nothing patched");
        };
        assert!(message.contains("program"), "{message}");
    }

    #[test]
    fn a_trim_reaches_what_is_applied_without_a_restart() {
        let engine = Engine::default();
        let replay = ReplayStore::default();
        let emit: Emit = Arc::new(|_| {});
        let outputs = Outputs::default();
        engine.apply(&outputs, &replay, emit.clone());
        engine.set_live(-6, true);
        let applied = engine.applied.lock().unwrap().clone().unwrap();
        assert_eq!(applied.shoutbox.trim_db, -6);
        assert!(applied.shoutbox.muted);
        // The same numbers saved afterwards are nothing new.
        let mut saved = outputs.clone();
        saved.shoutbox.trim_db = -6;
        saved.shoutbox.muted = true;
        assert!(engine.apply(&saved, &replay, emit).is_none());
        assert_eq!(engine.current_status().state, ShoutboxState::Off);
    }

    /// The whole shoutbox on a loopback device, which is silent:
    /// `SHOUTBOX_DEVICE="BlackHole 16ch" cargo test -p app shoutbox_on_a_real_device -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs a loopback audio device"]
    fn shoutbox_on_a_real_device() {
        let device = std::env::var("SHOUTBOX_DEVICE").unwrap_or_else(|_| "BlackHole 16ch".into());
        let input = |channel| InputRoute {
            device: device.clone(),
            device_id: String::new(),
            channel,
        };
        let config = Shoutbox {
            enabled: true,
            program: input(0),
            reference: input(1),
            output: OutputRoute {
                device: device.clone(),
                device_id: String::new(),
                channels: vec![2],
            },
            ..Shoutbox::default()
        };
        let seen = Arc::new(Mutex::new(Vec::<ShoutboxStatus>::new()));
        let emit: Emit = {
            let seen = seen.clone();
            Arc::new(move |s| seen.lock().unwrap().push(s))
        };
        let runner = Runner::start(config, Arc::new(Mutex::new(ShoutboxStatus::off())), emit);
        // Opening enumerates the devices a few times, which CoreAudio takes
        // its time over.
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline
            && !seen
                .lock()
                .unwrap()
                .iter()
                .any(|s| s.state != ShoutboxState::Starting)
        {
            std::thread::sleep(Duration::from_millis(50));
        }
        std::thread::sleep(Duration::from_millis(500));
        runner.stop();

        let seen = seen.lock().unwrap();
        for s in seen.iter().take(3) {
            println!("{s:?}");
        }
        assert!(
            seen.iter().any(|s| s.state == ShoutboxState::Running),
            "never ran: {:?}",
            seen.iter().find(|s| s.message.is_some())
        );
        assert_eq!(seen.last().map(|s| s.state), Some(ShoutboxState::Off));
    }
}
