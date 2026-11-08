//! The audio behind the bubbles, kept for a while so it can be heard again
//! (P22).
//!
//! The segmenter already cuts exactly the audio whisper is given for a
//! bubble, at 16 kHz mono; keeping it after the transcript comes back is a
//! buffer, not a pipeline. And because it is the segment and not the raw
//! channel, it plays what passed the speech gate — a bubble whose audio starts
//! mid-word says the gate opened late, not that whisper misheard.
//!
//! **Memory only, bounded twice.** A limit in minutes of *speech* — silence
//! never becomes a segment — and a hard byte ceiling behind it; the oldest
//! goes first. Nothing reaches the disk, and a killed app takes the audio with
//! it, exactly as it takes the chat (P10). Clearing the chat clears this too:
//! it is the same crew communication.

use audio_core::SAMPLE_RATE;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

/// The ceiling behind the limit in minutes: 128 MB of `i16`, a little over an
/// hour of speech. A setting of 120 minutes is answered by this rather than
/// by the machine's memory.
pub const MAX_BYTES: usize = 128 * 1024 * 1024;

/// What identifies one segment's audio — the same three numbers that identify
/// its transcript. See `session::NEXT_SESSION_ID` for why all three.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "replay.ts")]
pub struct ClipKey {
    pub session_id: u64,
    pub stream_id: u16,
    pub seq: u64,
}

/// The oldest audio still held, as a line through the chat: everything that
/// ended before it is gone (P22).
///
/// One line rather than a flag per bubble, because the store drops oldest
/// first across every channel — so what is gone is exactly what lies before
/// the oldest clip, and one event moves one line instead of touching every
/// bubble in every view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "replay.ts")]
pub struct Horizon {
    /// Every session before this one has lost its audio.
    pub session_id: u64,
    /// In this session, every segment that ended before this has lost it.
    pub t_end_ms: u64,
}

struct Clip {
    t_start_ms: u64,
    t_end_ms: u64,
    pcm: Vec<i16>,
}

#[derive(Default)]
struct Inner {
    clips: HashMap<ClipKey, Clip>,
    /// Insertion order, which is the order they are dropped in.
    order: VecDeque<ClipKey>,
    samples: usize,
    /// Samples the limit in minutes allows.
    limit: usize,
    /// Where the line was last drawn, so a change can be announced once.
    horizon: Option<Horizon>,
}

/// The store. Shared by the processing thread that fills it, the aggregator
/// that empties what was not speech, and the commands that play from it.
pub struct ReplayStore {
    inner: Mutex<Inner>,
}

impl Default for ReplayStore {
    fn default() -> Self {
        let store = Self {
            inner: Mutex::new(Inner::default()),
        };
        store.set_keep_minutes(crate::config::Listen::default().keep_minutes);
        store
    }
}

impl ReplayStore {
    /// How many minutes of speech are kept. Lowered, the excess goes now.
    /// Returns the new horizon if that moved it.
    pub fn set_keep_minutes(&self, minutes: u32) -> Option<Horizon> {
        let mut inner = self.inner.lock().ok()?;
        inner.limit = (minutes as usize * 60 * SAMPLE_RATE as usize).min(MAX_BYTES / 2);
        evict(&mut inner)
    }

    /// Keeps a segment's audio. Returns the new horizon if keeping it pushed
    /// older audio out.
    pub fn insert(
        &self,
        key: ClipKey,
        t_start_ms: u64,
        t_end_ms: u64,
        pcm: &[f32],
    ) -> Option<Horizon> {
        let mut inner = self.inner.lock().ok()?;
        if inner.limit == 0 {
            return None;
        }
        let pcm: Vec<i16> = pcm
            .iter()
            .map(|s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
            .collect();
        inner.samples += pcm.len();
        if let Some(old) = inner.clips.insert(
            key,
            Clip {
                t_start_ms,
                t_end_ms,
                pcm,
            },
        ) {
            inner.samples -= old.pcm.len();
        } else {
            inner.order.push_back(key);
        }
        evict(&mut inner)
    }

    /// Forgets a segment that turned out not to be speech, or was dropped
    /// under overload: no bubble will ever ask for it.
    pub fn remove(&self, key: &ClipKey) {
        if let Ok(mut inner) = self.inner.lock() {
            if let Some(clip) = inner.clips.remove(key) {
                inner.samples -= clip.pcm.len();
                inner.order.retain(|k| k != key);
            }
        }
    }

    pub fn contains(&self, key: &ClipKey) -> bool {
        self.inner
            .lock()
            .map(|inner| inner.clips.contains_key(key))
            .unwrap_or(false)
    }

    /// Forgets everything — the chat was cleared.
    pub fn clear(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            let limit = inner.limit;
            *inner = Inner {
                limit,
                ..Inner::default()
            };
        }
    }

    /// Where the line through the chat is, or `None` while nothing has been
    /// dropped.
    pub fn horizon(&self) -> Option<Horizon> {
        self.inner.lock().ok().and_then(|inner| inner.horizon)
    }

    /// The audio of a bubble: its segments in order, as 16 kHz mono.
    ///
    /// A forced cut hands the next segment ~500 ms of the audio before it
    /// (P3), so two neighbours overlap by exactly that. The later one is
    /// trimmed by the overlap its timestamps say it has: played back to back
    /// they would repeat half a second at every seam. `None` if any part is
    /// gone — half a bubble played as though it were all of it would be
    /// worse than none.
    pub fn assemble(&self, keys: &[ClipKey]) -> Option<Vec<f32>> {
        let inner = self.inner.lock().ok()?;
        let mut clips: Vec<(&ClipKey, &Clip)> = keys
            .iter()
            .map(|key| inner.clips.get(key).map(|clip| (key, clip)))
            .collect::<Option<_>>()?;
        clips.sort_by_key(|(key, clip)| (key.session_id, key.stream_id, clip.t_start_ms, key.seq));

        let mut out: Vec<f32> = Vec::new();
        let mut previous: Option<(&ClipKey, u64)> = None;
        for (key, clip) in clips {
            let skip = match previous {
                Some((before, end))
                    if before.session_id == key.session_id && before.stream_id == key.stream_id =>
                {
                    let overlap_ms = end.saturating_sub(clip.t_start_ms);
                    ((overlap_ms * SAMPLE_RATE as u64 / 1000) as usize).min(clip.pcm.len())
                }
                _ => 0,
            };
            out.extend(clip.pcm[skip..].iter().map(|&s| s as f32 / i16::MAX as f32));
            previous = Some((key, clip.t_end_ms));
        }
        (!out.is_empty()).then_some(out)
    }
}

/// Drops the oldest until both limits hold. Returns the horizon if it moved.
fn evict(inner: &mut Inner) -> Option<Horizon> {
    let ceiling = inner.limit.min(MAX_BYTES / 2);
    let mut moved = false;
    while inner.samples > ceiling {
        let Some(key) = inner.order.pop_front() else {
            break;
        };
        if let Some(clip) = inner.clips.remove(&key) {
            inner.samples -= clip.pcm.len();
            let line = Horizon {
                session_id: key.session_id,
                t_end_ms: clip.t_end_ms + 1,
            };
            // Monotonic: a clip dropped out of order must not pull the line
            // back over audio that is already gone.
            if inner
                .horizon
                .is_none_or(|h| (line.session_id, line.t_end_ms) > (h.session_id, h.t_end_ms))
            {
                inner.horizon = Some(line);
                moved = true;
            }
        }
    }
    moved.then_some(inner.horizon).flatten()
}

/// A mono 16-bit WAV of `samples` at `rate`, for a browser to play (P16).
pub fn wav(samples: &[f32], rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for &s in samples {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).to_le_bytes());
    }
    out
}

/// The samples of a WAV [`wav`] wrote, back at the pipeline's rate — what an
/// instance reading along is sent for a line it wants to hear (P29).
///
/// Mono 16-bit PCM and nothing else: this reads what another instance of this
/// app writes, not WAV files in general. `None` for anything else.
pub fn samples_of_wav(bytes: &[u8]) -> Option<Vec<f32>> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let mut at = 12;
    let mut rate = None;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let len = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().ok()?) as usize;
        let body = bytes.get(at + 8..(at + 8).checked_add(len)?.min(bytes.len()))?;
        if id == b"fmt " && body.len() >= 16 {
            let format = u16::from_le_bytes([body[0], body[1]]);
            let channels = u16::from_le_bytes([body[2], body[3]]);
            let bits = u16::from_le_bytes([body[14], body[15]]);
            if format != 1 || channels != 1 || bits != 16 {
                return None;
            }
            rate = Some(u32::from_le_bytes(body[4..8].try_into().ok()?));
        } else if id == b"data" {
            let rate = rate?;
            let samples: Vec<f32> = body
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| i16::from_le_bytes(*pair) as f32 / i16::MAX as f32)
                .collect();
            if rate == SAMPLE_RATE {
                return Some(samples);
            }
            return audio_core::resample::convert(&samples, rate, SAMPLE_RATE).ok();
        }
        at += 8 + len + (len & 1);
    }
    None
}

/// The other direction of [`base64`]. `None` for anything that is not
/// standard base64.
pub fn unbase64(text: &str) -> Option<Vec<u8>> {
    fn value(c: u8) -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    }
    let text = text.trim().as_bytes();
    if !text.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    for chunk in text.chunks(4) {
        let pad = chunk.iter().rev().take_while(|&&c| c == b'=').count();
        if pad > 2 {
            return None;
        }
        let mut n = 0u32;
        for &c in &chunk[..4 - pad] {
            n = n << 6 | value(c)?;
        }
        n <<= 6 * pad as u32;
        let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        out.extend_from_slice(&bytes[..3 - pad]);
    }
    Some(out)
}

/// Standard base64, for a WAV travelling inside a JSON answer. Written out
/// rather than a dependency: it is twenty lines and one direction.
pub fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            if i <= chunk.len() {
                out.push(TABLE[((n >> shift) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(stream: u16, seq: u64) -> ClipKey {
        ClipKey {
            session_id: 1,
            stream_id: stream,
            seq,
        }
    }

    fn seconds(s: f32, value: f32) -> Vec<f32> {
        vec![value; (s * SAMPLE_RATE as f32) as usize]
    }

    #[test]
    fn a_bubble_plays_its_segments_in_order_without_repeating_the_overlap() {
        let store = ReplayStore::default();
        // Two segments of a forced cut: the second starts 500 ms before the
        // first ends, and those 500 ms are in both.
        store.insert(key(0, 1), 6_000, 12_000, &seconds(6.0, 0.25));
        store.insert(key(0, 0), 0, 6_000, &seconds(6.0, 0.5));

        let audio = store.assemble(&[key(0, 1), key(0, 0)]).unwrap();
        // 6 s + 6 s, since these two do not overlap…
        assert_eq!(audio.len(), 12 * SAMPLE_RATE as usize);
        assert!(audio[0] > 0.4 && audio[audio.len() - 1] < 0.3);

        store.insert(key(0, 2), 11_500, 17_500, &seconds(6.0, 0.1));
        let audio = store.assemble(&[key(0, 1), key(0, 2)]).unwrap();
        // …and these two do, by half a second.
        assert_eq!(audio.len(), (11.5 * SAMPLE_RATE as f32) as usize);
    }

    #[test]
    fn a_bubble_with_a_part_missing_does_not_play_half_of_itself() {
        let store = ReplayStore::default();
        store.insert(key(0, 0), 0, 1_000, &seconds(1.0, 0.5));
        assert!(store.assemble(&[key(0, 0), key(0, 1)]).is_none());
    }

    #[test]
    fn the_oldest_goes_first_and_the_line_moves_with_it() {
        let store = ReplayStore::default();
        store.set_keep_minutes(1);
        assert_eq!(store.horizon(), None);
        for seq in 0..70 {
            store.insert(
                key((seq % 3) as u16, seq),
                seq * 1_000,
                seq * 1_000 + 1_000,
                &seconds(1.0, 0.1),
            );
        }
        // 70 s of speech into a one-minute store: the first ten are gone.
        assert!(!store.contains(&key(0, 0)));
        assert!(!store.contains(&key(0, 9)));
        assert!(store.contains(&key(1, 10)));
        assert_eq!(
            store.horizon(),
            Some(Horizon {
                session_id: 1,
                t_end_ms: 10_001
            })
        );
    }

    #[test]
    fn nothing_is_kept_when_the_limit_is_zero_and_clearing_empties_it() {
        let store = ReplayStore::default();
        store.set_keep_minutes(0);
        store.insert(key(0, 0), 0, 1_000, &seconds(1.0, 0.1));
        assert!(!store.contains(&key(0, 0)));

        store.set_keep_minutes(10);
        store.insert(key(0, 0), 0, 1_000, &seconds(1.0, 0.1));
        store.clear();
        assert!(!store.contains(&key(0, 0)));
    }

    #[test]
    fn the_wav_says_what_it_holds() {
        let wav = wav(&[0.0, 1.0, -1.0], 16_000);
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(wav.len(), 44 + 6);
        assert_eq!(&wav[44..46], &0i16.to_le_bytes());
        assert_eq!(&wav[46..48], &i16::MAX.to_le_bytes());
    }

    #[test]
    fn base64_is_the_standard_alphabet_with_padding() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn a_wav_survives_the_trip_to_another_instance() {
        let samples: Vec<f32> = (0..321).map(|i| ((i as f32) / 50.0).sin() * 0.5).collect();
        let text = base64(&wav(&samples, SAMPLE_RATE));
        let back = samples_of_wav(&unbase64(&text).expect("base64")).expect("a wav");
        assert_eq!(back.len(), samples.len());
        for (a, b) in back.iter().zip(&samples) {
            assert!((a - b).abs() < 1e-3);
        }
        assert_eq!(unbase64("YQ==").as_deref(), Some(&b"a"[..]));
        assert_eq!(unbase64("YWI=").as_deref(), Some(&b"ab"[..]));
        assert!(unbase64("Y").is_none());
    }
}
