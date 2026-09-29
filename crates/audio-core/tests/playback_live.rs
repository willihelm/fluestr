//! The way out, against the machine's real devices (P22, P26).
//!
//! `cargo test -p audio-core --test playback_live -- --ignored --nocapture`
//!
//! Plays *silence* on the default output — what is checked is that the
//! stream opens, the callback pulls from the player and the player reports
//! the cue finished, not what anybody hears.

use audio_core::{playback, AudioBackend, CpalBackend, OutputConfig};
use std::time::{Duration, Instant};

#[test]
#[ignore = "needs the machine's audio devices"]
fn a_replay_runs_to_its_end_on_the_default_output() {
    let backend = CpalBackend::new();
    let outputs = backend.output_devices().expect("listing outputs");
    for info in &outputs {
        println!(
            "{:40} {:3} ch {:6} Hz {}{}",
            info.name,
            info.channels,
            info.default_sample_rate,
            info.id,
            if info.is_default { "  (default)" } else { "" }
        );
    }
    let default = backend.output_device(None).expect("a default output");

    let (player, mut handle) = playback::player();
    let (events, _events) = std::sync::mpsc::channel();
    let session = backend
        .open_output(
            OutputConfig {
                device: default.id.clone(),
                channels: vec![0],
                sample_rate: None,
            },
            Box::new(player),
            events,
        )
        .expect("opening the default output");

    let rate = session.sample_rate();
    handle.play(1, vec![0.0; rate as usize / 4]).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline && handle.state().playing() != Some(1) {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        handle.state().playing(),
        Some(1),
        "the callback never took the cue"
    );
    while Instant::now() < deadline && handle.state().playing().is_some() {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(handle.state().playing(), None, "the replay never finished");
    session.stop().unwrap();
}

/// What the input pickers are offered, beside what the outputs are:
/// `cargo test -p audio-core --test playback_live inputs_on_this_machine -- --ignored --nocapture`.
#[test]
#[ignore = "needs the machine's audio devices"]
fn inputs_on_this_machine() {
    for info in CpalBackend::new().input_devices().expect("listing inputs") {
        println!(
            "{:40} {:3} ch {:6} Hz {}{}",
            info.name,
            info.channels,
            info.default_sample_rate,
            info.id,
            if info.is_default { "  (default)" } else { "" }
        );
    }
}
