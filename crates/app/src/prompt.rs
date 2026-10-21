//! What whisper is told before each segment (P15, P25).
//!
//! Whisper does not follow instructions. Its `initial_prompt` is read as the
//! transcript that came *before* the segment, and nothing else — so everything
//! composed here is text that should look like the channel: the house's
//! vocabulary and, when it is safe to, the sentence the same channel said
//! last.
//!
//! # Composed when a worker takes the segment, not when it is cut
//!
//! The previous sentence is the result of the segment before, and that result
//! may not exist yet when the segmenter closes the next one. So the prompt is
//! put together by the pool, at the moment a worker takes the job — see
//! [`crate::pool::PoolConfig::prompter`]. Without a backlog that is always late
//! enough: a segment cannot close sooner than one silence timeout plus its own
//! speech after its predecessor, and the predecessor decodes in ~330 ms.
//!
//! When the predecessor's text is not there yet — the pool is behind, or the
//! predecessor was requeued after a crash — the segment goes **without** it
//! rather than waiting. Waiting would serialise the stream exactly when the
//! pool is behind, and latency comes first.
//!
//! # Why not whisper's own context
//!
//! The sidecar sets `no_context`: a worker's state is shared by every stream
//! it decodes, and whisper's built-in carry would hand the lighting channel's
//! sentence to the stage manager's next segment. Carrying it here, per stream,
//! is the same idea with control over it — a gap after which it lapses, a
//! language change that drops it, and only text that actually reached the
//! chat.

use audio_core::Segment;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

/// How long after the end of a segment its text is still worth carrying into
/// the next one on the same channel.
///
/// Seconds rather than minutes: within this the same person is usually still
/// talking, or being answered on the same line. Beyond it the sentence belongs
/// to a different exchange, and priming the next one with it is how a word
/// transcribed wrong once comes out wrong again an hour later.
pub const CARRY_WITHIN: Duration = Duration::from_secs(10);

/// The most of the previous sentence that is carried, in characters.
///
/// About fifty tokens — a quarter of whisper's 224-token window. The end of the
/// sentence is what is kept: it is what the next segment continues, and an
/// over-long prompt loses its *front*, which is where the vocabulary is.
pub const CARRY_CHARS: usize = 200;

/// Composes the prompt for every segment of one session.
///
/// Shared between the session, which edits it — the switch on the Detection
/// page, a language change — the aggregator, which tells it
/// what each channel last said, and the pool, which reads it once per decode.
/// One lock around all of it: every access is a handful of string operations
/// beside a decode that takes hundreds of milliseconds.
#[derive(Debug)]
pub struct Prompter {
    inner: Mutex<Inner>,
}

#[derive(Debug)]
struct Inner {
    vocabulary: Option<String>,
    /// Whether the previous sentence is carried at all — the switch on the
    /// Detection page.
    carry: bool,
    previous: HashMap<u16, Previous>,
}

/// What one channel said last, as it reached the chat.
#[derive(Debug, Clone)]
struct Previous {
    seq: u64,
    t_end: Duration,
    /// Whisper's language for it. Compared against the next segment's, so a
    /// channel put onto another language does not carry German into English.
    language: Option<String>,
    text: String,
}

impl Prompter {
    pub fn new(vocabulary: Option<String>, carry: bool) -> Self {
        Self {
            inner: Mutex::new(Inner {
                vocabulary: non_empty(vocabulary.as_deref()),
                carry,
                previous: HashMap::new(),
            }),
        }
    }

    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().expect("prompter poisoned")
    }

    /// Turns carrying the previous sentence on or off, for every channel.
    ///
    /// Turning it off also forgets what was held, so that turning it back on
    /// later does not carry a sentence from before the switch was touched.
    pub fn set_carry(&self, carry: bool) {
        let mut inner = self.inner();
        inner.carry = carry;
        if !carry {
            inner.previous.clear();
        }
    }

    /// Drops what one channel said last — its language changed, or the
    /// channel was handed to somebody else.
    pub fn forget(&self, stream: u16) {
        self.inner().previous.remove(&stream);
    }

    /// Records a transcript that reached the chat.
    ///
    /// Only those: a segment that came back empty, as an annotation, or not at
    /// all is never recorded, and because [`Prompter::compose`] carries only
    /// the *direct* predecessor, the segment after it goes without rather than
    /// being handed the one before that.
    pub fn remember(
        &self,
        stream: u16,
        seq: u64,
        t_end: Duration,
        language: Option<&str>,
        text: &str,
    ) {
        let mut inner = self.inner();
        if !inner.carry {
            return;
        }
        let Some(text) = non_empty(Some(text)) else {
            return;
        };
        // Out-of-order arrival cannot make an older sentence replace a newer
        // one: the aggregator hands results over in order, and this guards the
        // assumption rather than relying on it.
        if inner
            .previous
            .get(&stream)
            .is_some_and(|held| held.seq > seq)
        {
            return;
        }
        inner.previous.insert(
            stream,
            Previous {
                seq,
                t_end,
                language: language.map(str::to_string),
                text,
            },
        );
    }

    /// The prompt for one segment, or `None` for no prompt at all.
    ///
    /// Vocabulary first, then the previous sentence. The order is not cosmetic: whisper drops an over-long prompt
    /// from the front, so what is most specific to this segment goes last and
    /// survives.
    pub fn compose(&self, segment: &Segment) -> Option<String> {
        let inner = self.inner();
        let stream = segment.stream_id.0;

        let carried = inner
            .carry
            .then(|| inner.previous.get(&stream))
            .flatten()
            .filter(|held| held.seq + 1 == segment.seq)
            .filter(|held| segment.t_start.saturating_sub(held.t_end) <= CARRY_WITHIN)
            .filter(|held| {
                match (
                    held.language.as_deref(),
                    segment.language_mode.whisper_language(),
                ) {
                    (Some(was), Some(is)) => was.eq_ignore_ascii_case(is),
                    _ => true,
                }
            })
            .map(|held| tail(&held.text, CARRY_CHARS));

        join([inner.vocabulary.as_deref(), carried])
    }
}

fn non_empty(text: Option<&str>) -> Option<String> {
    text.map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

/// The last `max` characters of `text`, starting at a word.
fn tail(text: &str, max: usize) -> &str {
    let count = text.chars().count();
    if count <= max {
        return text;
    }
    let start = text
        .char_indices()
        .nth(count - max)
        .map_or(0, |(index, _)| index);
    let rest = &text[start..];
    // Past the word the cut landed in, so the prompt does not begin with half
    // of one — whisper would read it as a word that was said.
    match rest.find(char::is_whitespace) {
        Some(space) => rest[space..].trim_start(),
        None => rest,
    }
}

/// Joins the parts into one text that reads as a transcript.
///
/// A part that does not end a sentence is closed with a full stop, so the
/// vocabulary's last term does not run into the carried sentence's first word
/// as if they were one phrase.
fn join<'a>(parts: impl IntoIterator<Item = Option<&'a str>>) -> Option<String> {
    let mut out = String::new();
    for part in parts.into_iter().flatten() {
        if !out.is_empty() {
            if !out.ends_with(['.', '!', '?', '…', ':', ';']) {
                out.push('.');
            }
            out.push(' ');
        }
        out.push_str(part);
    }
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use audio_core::{LanguageCode, LanguageMode, StreamId};

    fn segment(stream: u16, seq: u64, t_start_ms: u64) -> Segment {
        segment_in(stream, seq, t_start_ms, "de")
    }

    fn segment_in(stream: u16, seq: u64, t_start_ms: u64, language: &str) -> Segment {
        Segment {
            stream_id: StreamId(stream),
            seq,
            t_start: Duration::from_millis(t_start_ms),
            t_end: Duration::from_millis(t_start_ms + 2_000),
            lag: Duration::ZERO,
            pcm: Vec::new(),
            language_mode: LanguageMode::Fixed {
                language: LanguageCode::new(language),
            },
            initial_prompt: None,
        }
    }

    fn at(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    #[test]
    fn nothing_configured_is_no_prompt_at_all() {
        let prompter = Prompter::new(None, true);
        assert_eq!(prompter.compose(&segment(0, 0, 0)), None);
    }

    #[test]
    fn vocabulary_and_previous_sentence_in_that_order() {
        let prompter = Prompter::new(Some("Standby, Cue, Go".into()), true);
        prompter.remember(3, 4, at(10_000), Some("de"), "Preset 12 steht");

        assert_eq!(
            prompter.compose(&segment(3, 5, 11_000)).as_deref(),
            Some("Standby, Cue, Go. Preset 12 steht")
        );
    }

    #[test]
    fn only_the_direct_predecessor_is_carried() {
        // Segment 5 never reached the chat — empty, dropped, lost — so 6 gets
        // nothing rather than 4's sentence.
        let prompter = Prompter::new(None, true);
        prompter.remember(0, 4, at(1_000), Some("de"), "Achtung Bühne");
        assert_eq!(prompter.compose(&segment(0, 6, 2_000)), None);
    }

    #[test]
    fn a_predecessor_still_decoding_is_not_waited_for() {
        // 5 has not come back yet when 6 is taken: 6 goes without.
        let prompter = Prompter::new(Some("Cue".into()), true);
        prompter.remember(0, 4, at(1_000), Some("de"), "Achtung Bühne");
        assert_eq!(
            prompter.compose(&segment(0, 6, 2_000)).as_deref(),
            Some("Cue")
        );
    }

    #[test]
    fn the_previous_sentence_lapses_after_a_pause() {
        let prompter = Prompter::new(None, true);
        prompter.remember(0, 0, at(1_000), Some("de"), "Achtung Bühne");
        let late = 1_000 + CARRY_WITHIN.as_millis() as u64 + 1;
        assert_eq!(prompter.compose(&segment(0, 1, late)), None);
        let in_time = 1_000 + CARRY_WITHIN.as_millis() as u64;
        assert!(prompter.compose(&segment(0, 1, in_time)).is_some());
    }

    #[test]
    fn a_language_change_is_not_carried_across() {
        let prompter = Prompter::new(None, true);
        prompter.remember(0, 0, at(1_000), Some("de"), "Achtung Bühne");
        assert_eq!(prompter.compose(&segment_in(0, 1, 1_500, "en")), None);
    }

    #[test]
    fn switched_off_carries_nothing_and_forgets_what_it_held() {
        let prompter = Prompter::new(None, true);
        prompter.remember(0, 0, at(1_000), Some("de"), "Achtung Bühne");
        prompter.set_carry(false);
        assert_eq!(prompter.compose(&segment(0, 1, 1_500)), None);

        // Back on: the sentence from before the switch is gone, not revived.
        prompter.set_carry(true);
        assert_eq!(prompter.compose(&segment(0, 1, 1_500)), None);

        // And while off, nothing new is recorded either.
        prompter.set_carry(false);
        prompter.remember(0, 1, at(2_000), Some("de"), "Bühne frei");
        prompter.set_carry(true);
        assert_eq!(prompter.compose(&segment(0, 2, 2_500)), None);
    }

    #[test]
    fn forgetting_a_channel_leaves_the_others() {
        let prompter = Prompter::new(None, true);
        prompter.remember(0, 0, at(1_000), Some("de"), "Achtung Bühne");
        prompter.remember(1, 0, at(1_000), Some("de"), "Licht bereit");
        prompter.forget(0);
        assert_eq!(prompter.compose(&segment(0, 1, 1_500)), None);
        assert_eq!(
            prompter.compose(&segment(1, 1, 1_500)).as_deref(),
            Some("Licht bereit")
        );
    }

    #[test]
    fn a_long_sentence_is_carried_from_its_end_starting_at_a_word() {
        let long = format!("{} Standby Licht zwölf", "wort ".repeat(60));
        let carried = tail(&long, 30);
        assert!(carried.chars().count() <= 30);
        assert!(carried.ends_with("Standby Licht zwölf"));
        assert!(carried.starts_with("wort") || carried.starts_with("Standby"));
    }

    #[test]
    fn the_cut_never_splits_an_umlaut() {
        let text = "ä".repeat(300);
        // One unbroken "word": no space to move past, but still valid UTF-8.
        assert_eq!(tail(&text, 10).chars().count(), 10);
    }

    #[test]
    fn a_part_that_ends_a_sentence_is_not_given_a_second_stop() {
        assert_eq!(
            join([Some("Kommandos: Cue, Go."), Some("Licht")]).as_deref(),
            Some("Kommandos: Cue, Go. Licht")
        );
    }
}
