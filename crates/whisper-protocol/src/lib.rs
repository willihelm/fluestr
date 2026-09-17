//! Length-prefixed protocol between the app and the sidecar.
//!
//! Deliberately its own crate: both ends of the pipe need it — the worker
//! reads requests and writes responses, the worker pool (P5) does the
//! opposite. A shared crate keeps the definitions in one place and makes
//! round-trip tests possible.
//!
//! Frame format, little-endian throughout:
//!
//! ```text
//! Request:  [u32 json_len][json bytes][u32 pcm_bytes][f32-LE samples]
//! Response: [u32 json_len][json bytes]
//! ```
//!
//! PCM travels **as binary**, not base64 — across 8 streams that saves
//! copying and encoding work on the latency-critical path.

use anyhow::{bail, Context, Result};
use audio_core::LanguageMode;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

/// Upper bound for a PCM block. At 16 kHz mono f32, 16 MiB is a good four
/// minutes — far beyond any realistic segment (max ~6 s) and therefore an
/// effective guard against corrupt length prefixes.
const MAX_PCM_BYTES: u32 = 16 * 1024 * 1024;

/// Upper bound for the JSON header.
const MAX_JSON_BYTES: u32 = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    /// Transcribe one segment. Immediately followed by the PCM block.
    Transcribe {
        stream_id: u16,
        seq: u64,
        #[serde(default)]
        language_mode: LanguageMode,
        #[serde(default)]
        initial_prompt: Option<String>,
    },
    /// Ask whisper.cpp for its full language list. Feeds the UI picker so no
    /// language list is hard-coded anywhere.
    ListLanguages,
    /// Shut down cleanly.
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    /// Model loaded, the worker accepts work.
    Ready {
        model: String,
        /// Load time. On the very first start on a machine this includes the
        /// one-off Metal shader compilation (measured at ~8 s); afterwards the
        /// system-wide cache kicks in and it is ~0.1 s.
        load_ms: u32,
        gpu: bool,
    },
    Transcript {
        stream_id: u16,
        seq: u64,
        text: String,
        detected_language: Option<String>,
        /// Only populated in auto mode — the basis for the language
        /// stabiliser (P3b).
        language_probabilities: Option<Vec<(String, f32)>>,
        decode_ms: u32,
    },
    Languages {
        languages: Vec<Language>,
    },
    Error {
        /// Set for a segment-level failure so the app can attribute the error
        /// and drop that segment instead of waiting forever.
        stream_id: Option<u16>,
        seq: Option<u64>,
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Language {
    /// ISO 639-1, e.g. "de".
    pub code: String,
    /// Spelled-out name, e.g. "german".
    pub name: String,
}

/// Reads one request along with its PCM block.
///
/// Returns `Ok(None)` when stdin was closed cleanly.
pub fn read_request<R: Read>(reader: &mut R) -> Result<Option<(Request, Vec<f32>)>> {
    let json_len = match read_u32(reader)? {
        Some(n) => n,
        None => return Ok(None),
    };
    if json_len > MAX_JSON_BYTES {
        bail!("JSON header too large: {json_len} bytes (max {MAX_JSON_BYTES})");
    }

    let mut json = vec![0u8; json_len as usize];
    reader
        .read_exact(&mut json)
        .context("incomplete JSON header")?;
    let request: Request = serde_json::from_slice(&json).context("malformed JSON header")?;

    // Only Transcribe carries PCM.
    if !matches!(request, Request::Transcribe { .. }) {
        return Ok(Some((request, Vec::new())));
    }

    let pcm_bytes = read_u32(reader)?.context("missing PCM length")?;
    if pcm_bytes > MAX_PCM_BYTES {
        bail!("PCM block too large: {pcm_bytes} bytes (max {MAX_PCM_BYTES})");
    }
    if pcm_bytes % 4 != 0 {
        bail!("PCM length {pcm_bytes} is not a multiple of 4 (f32)");
    }

    let mut raw = vec![0u8; pcm_bytes as usize];
    reader.read_exact(&mut raw).context("incomplete PCM")?;
    // The length was checked to be a multiple of 4 above, so the remainder is empty.
    let pcm = raw
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| f32::from_le_bytes(*c))
        .collect();

    Ok(Some((request, pcm)))
}

pub fn write_response<W: Write>(writer: &mut W, response: &Response) -> Result<()> {
    let json = serde_json::to_vec(response).context("serialising response")?;
    writer.write_all(&(json.len() as u32).to_le_bytes())?;
    writer.write_all(&json)?;
    writer.flush()?;
    Ok(())
}

/// Counterpart to [`read_request`], used by tests and by the app side (P5).
pub fn write_request<W: Write>(writer: &mut W, request: &Request, pcm: &[f32]) -> Result<()> {
    let json = serde_json::to_vec(request).context("serialising request")?;
    writer.write_all(&(json.len() as u32).to_le_bytes())?;
    writer.write_all(&json)?;

    if matches!(request, Request::Transcribe { .. }) {
        writer.write_all(&((pcm.len() * 4) as u32).to_le_bytes())?;
        for sample in pcm {
            writer.write_all(&sample.to_le_bytes())?;
        }
    }
    writer.flush()?;
    Ok(())
}

pub fn read_response<R: Read>(reader: &mut R) -> Result<Option<Response>> {
    let json_len = match read_u32(reader)? {
        Some(n) => n,
        None => return Ok(None),
    };
    if json_len > MAX_JSON_BYTES {
        bail!("response too large: {json_len} bytes");
    }
    let mut json = vec![0u8; json_len as usize];
    reader
        .read_exact(&mut json)
        .context("incomplete response")?;
    Ok(Some(serde_json::from_slice(&json)?))
}

/// Reads a little-endian u32; `None` on a clean EOF, an error on a partial prefix.
fn read_u32<R: Read>(reader: &mut R) -> Result<Option<u32>> {
    let mut buf = [0u8; 4];
    let mut filled = 0;
    while filled < 4 {
        match reader.read(&mut buf[filled..])? {
            0 if filled == 0 => return Ok(None),
            0 => bail!("EOF in the middle of a length prefix ({filled} of 4 bytes)"),
            n => filled += n,
        }
    }
    Ok(Some(u32::from_le_bytes(buf)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use audio_core::LanguageCode;
    use std::io::Cursor;

    #[test]
    fn transcribe_round_trip_preserves_pcm() {
        let request = Request::Transcribe {
            stream_id: 3,
            seq: 7,
            language_mode: LanguageMode::default(),
            initial_prompt: Some("Standby, Cue, Fade out".into()),
        };
        let pcm = vec![0.0f32, 0.5, -0.5, 1.0, -1.0];

        let mut buf = Vec::new();
        write_request(&mut buf, &request, &pcm).unwrap();

        let (decoded, decoded_pcm) = read_request(&mut Cursor::new(buf)).unwrap().unwrap();
        assert_eq!(decoded_pcm, pcm);
        match decoded {
            Request::Transcribe {
                stream_id,
                seq,
                initial_prompt,
                ..
            } => {
                assert_eq!(stream_id, 3);
                assert_eq!(seq, 7);
                assert_eq!(initial_prompt.as_deref(), Some("Standby, Cue, Fade out"));
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn auto_mode_survives_the_round_trip() {
        let request = Request::Transcribe {
            stream_id: 0,
            seq: 0,
            language_mode: LanguageMode::Auto {
                candidates: vec![LanguageCode::german(), LanguageCode::english()],
            },
            initial_prompt: None,
        };
        let mut buf = Vec::new();
        write_request(&mut buf, &request, &[]).unwrap();
        let (decoded, _) = read_request(&mut Cursor::new(buf)).unwrap().unwrap();
        match decoded {
            Request::Transcribe { language_mode, .. } => {
                assert!(language_mode.is_auto());
                assert_eq!(language_mode.candidates().len(), 2);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn commands_without_pcm_carry_no_block() {
        let mut buf = Vec::new();
        write_request(&mut buf, &Request::ListLanguages, &[]).unwrap();
        let (decoded, pcm) = read_request(&mut Cursor::new(buf)).unwrap().unwrap();
        assert!(matches!(decoded, Request::ListLanguages));
        assert!(pcm.is_empty());
    }

    #[test]
    fn a_clean_eof_is_not_an_error() {
        let mut empty = Cursor::new(Vec::new());
        assert!(read_request(&mut empty).unwrap().is_none());
    }

    #[test]
    fn a_partial_length_prefix_is_an_error() {
        // A truncated frame must not pass as EOF — otherwise the worker
        // silently swallows work.
        let mut truncated = Cursor::new(vec![1u8, 2]);
        assert!(read_request(&mut truncated).is_err());
    }

    #[test]
    fn a_truncated_pcm_block_is_an_error() {
        let mut buf = Vec::new();
        write_request(
            &mut buf,
            &Request::Transcribe {
                stream_id: 0,
                seq: 0,
                language_mode: LanguageMode::default(),
                initial_prompt: None,
            },
            &[1.0, 2.0, 3.0],
        )
        .unwrap();
        buf.truncate(buf.len() - 4); // one sample short
        assert!(read_request(&mut Cursor::new(buf)).is_err());
    }

    #[test]
    fn response_round_trip() {
        let response = Response::Transcript {
            stream_id: 2,
            seq: 99,
            // German on purpose: this is speech data, not code.
            text: "Standby für Cue fünf".into(),
            detected_language: Some("de".into()),
            language_probabilities: None,
            decode_ms: 412,
        };
        let mut buf = Vec::new();
        write_response(&mut buf, &response).unwrap();
        match read_response(&mut Cursor::new(buf)).unwrap().unwrap() {
            Response::Transcript {
                text, decode_ms, ..
            } => {
                assert_eq!(text, "Standby für Cue fünf");
                assert_eq!(decode_ms, 412);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }
}
