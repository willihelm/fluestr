//! The model download, against the real host (P11).
//!
//! Ignored by default, like every other test here that needs something the
//! machine may not have — in this case a network. `just models` runs them.
//!
//! The VAD model is the one they fetch: at 885 KB it is small enough to
//! download several times in a test run, and it is a real file from the real
//! host, which is the whole point. A mocked server would exercise this code
//! against an HTTP implementation nobody ships, and the two things most likely
//! to be wrong — the URL and the checksum — are exactly the two a mock
//! replaces.

use app_lib::models::{self, Stage};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

fn temp_dir(tag: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "intercom2chat-download-{tag}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("creating the test directory");
    dir
}

/// The small one, so these tests stay a few seconds rather than a few minutes.
fn vad() -> &'static models::Entry {
    models::entry(models::DEFAULT_VAD).expect("the VAD model is in the catalogue")
}

fn part_of(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{}.part", models::file_name(id)))
}

#[tokio::test]
#[ignore = "downloads from the network"]
async fn a_model_arrives_verified_and_installed() {
    let dir = temp_dir("fresh");
    let entry = vad();
    let stages = Arc::new(Mutex::new(Vec::new()));

    let recorder = Arc::clone(&stages);
    let path = models::download(
        entry,
        &dir,
        Arc::new(AtomicBool::new(false)),
        move |progress| recorder.lock().unwrap().push(progress.stage),
    )
    .await
    .expect("downloading the VAD model");

    assert_eq!(path, models::installed_path(&dir, entry.id));
    assert_eq!(
        std::fs::metadata(&path).unwrap().len(),
        entry.bytes,
        "the catalogue's size is the file's actual size"
    );
    // The checksum baked into the catalogue is the published one. A typo there
    // is otherwise only discovered on a real machine, ten minutes into a
    // download of the large model.
    assert!(models::verify(&path, entry.sha256).unwrap());

    let stages = stages.lock().unwrap();
    assert!(
        stages.contains(&Stage::Verifying),
        "the check has to be visible: hashing is not instant and a bar stuck \
         at 100 % looks like a hang — saw {stages:?}"
    );
    assert_eq!(stages.last(), Some(&Stage::Done));

    // Nothing half-written left behind. A `.part` next to a finished model
    // would be resumed into on the next attempt.
    assert!(!part_of(&dir, entry.id).exists());
}

#[tokio::test]
#[ignore = "downloads from the network"]
async fn an_interrupted_download_is_continued_rather_than_restarted() {
    // The reason the part file exists at all: 600 MB on a venue's network is
    // not one uninterrupted transfer, and starting over after each failure is
    // how somebody gives up and goes back to the terminal.
    let dir = temp_dir("resume");
    let entry = vad();
    let part = part_of(&dir, entry.id);

    // A previous attempt that got the first 400 KB. Taken from the real file
    // rather than invented, because a resume that splices the right bytes onto
    // the wrong ones is precisely what the checksum at the end has to catch —
    // and here it must *not* fire.
    let whole = reqwest::get(entry.url)
        .await
        .expect("fetching the reference copy")
        .bytes()
        .await
        .expect("reading the reference copy");
    assert_eq!(whole.len() as u64, entry.bytes);
    let cut = 400_000;
    std::fs::write(&part, &whole[..cut]).expect("writing the partial download");

    let first = Arc::new(Mutex::new(None));
    let recorder = Arc::clone(&first);
    let path = models::download(
        entry,
        &dir,
        Arc::new(AtomicBool::new(false)),
        move |progress| {
            let mut slot = recorder.lock().unwrap();
            if slot.is_none() {
                *slot = Some(progress.received);
            }
        },
    )
    .await
    .expect("resuming");

    // The bar picks up where it left off rather than at zero — which is also
    // the evidence that the existing bytes were kept rather than refetched.
    assert_eq!(
        *first.lock().unwrap(),
        Some(cut as u64),
        "a resumed download reports the bytes it already had"
    );
    assert!(models::verify(&path, entry.sha256).unwrap());
}

#[tokio::test]
#[ignore = "downloads from the network"]
async fn a_tampered_download_is_rejected_and_discarded() {
    // P11's acceptance criterion, and the reason the checksum is here: a
    // damaged model does not announce itself. whisper.cpp either refuses it
    // with a message about magic numbers, or loads it and transcribes noise.
    let dir = temp_dir("tampered");
    let entry = vad();
    let part = part_of(&dir, entry.id);

    // One byte short of the full size, so the download resumes, fetches that
    // last byte from the real host, and arrives at exactly the right *size*
    // with the wrong contents. Only the hash can tell the difference then,
    // which is the case the hash is here for — a part file of the full size is
    // discarded as debris before it ever reaches the check, so writing one
    // would test the wrong thing.
    std::fs::write(&part, vec![0xABu8; entry.bytes as usize - 1]).expect("writing the fake");
    let err = models::download(entry, &dir, Arc::new(AtomicBool::new(false)), |_| {})
        .await
        .map(|_| ())
        .expect_err("a file that is not the model must not be installed");

    let message = err.to_string();
    assert!(
        message.contains("checksum"),
        "the reason has to be the checksum, not the size: {message}"
    );
    assert!(
        !models::installed_path(&dir, entry.id).exists(),
        "a file that failed its checksum must never be installed"
    );
    assert!(
        !part.exists(),
        "and it must not be kept either — one resume away from being installed \
         is how a corrupt model survives every later attempt"
    );
}

#[tokio::test]
#[ignore = "downloads from the network"]
async fn cancelling_keeps_what_arrived() {
    let dir = temp_dir("cancel");
    let entry = vad();
    let cancel = Arc::new(AtomicBool::new(true));

    let err = models::download(entry, &dir, Arc::clone(&cancel), |_| {})
        .await
        .map(|_| ())
        .expect_err("a cancelled download does not produce a model");
    assert_eq!(err.to_string(), "cancelled");

    assert!(!models::installed_path(&dir, entry.id).exists());
    // The part file is deliberately left alone — pressing download again has
    // to continue, which is the entire reason cancelling is offered.
    assert!(
        part_of(&dir, entry.id).exists(),
        "cancelling must keep what arrived"
    );
}
