//! Collapsing whisper's repetition loops.
//!
//! Greedy decoding at a fixed temperature has one classic failure: the decoder
//! falls into a cycle and emits the same words until it runs out of token
//! budget — "das ist schon jetzt, das ist schon jetzt, …" forty times over.
//! It is not a transcription of anything; it is the sampler chasing its own
//! tail, usually on a segment whose audio ran out mid-phrase.
//!
//! The upstream cures all cost latency. Temperature fallback re-decodes the
//! same audio two to six times over, which is exactly what the sidecar is
//! built not to do (see `whisper-worker`), and the entropy and log-probability
//! thresholds that would trigger it do nothing while fallback is off. So the
//! loop is cut where it is cheap to cut: in the text, after the fact, where
//! the pattern is unmistakable.
//!
//! The rule is deliberately conservative, because real speech repeats too.
//! "Go, go, go" over an intercom is an instruction, not a defect, and losing
//! it would be the kind of silent damage this project does not accept — so a
//! short phrase has to repeat more often before it counts as a loop than a
//! long one, and a run that is collapsed keeps more than a single copy when
//! the repetition could plausibly have been spoken.

use std::borrow::Cow;

/// Longest phrase considered as a loop body, in words.
///
/// Beyond this the repetition is long enough to be a genuinely repeated
/// sentence — a cue read back, an instruction confirmed — and cutting it would
/// remove content rather than noise.
const MAX_PERIOD: usize = 8;

/// One word and where it sits in the original text.
struct Word {
    end: usize,
    /// What the comparison runs on: case-folded, so that the second copy of a
    /// phrase matches the first even when whisper capitalised it as the start
    /// of a sentence.
    key: String,
}

/// Cuts repetition loops out of a decoded segment.
///
/// Returns the text untouched — and borrowed, so the common case allocates
/// nothing — when there is no loop in it. An [`Cow::Owned`] result is
/// therefore also the signal that something was cut, which is what the caller
/// logs: a collapsed loop means the decode misbehaved, and that is worth a
/// line even though the chat now reads correctly.
pub fn collapse_repetitions(text: &str) -> Cow<'_, str> {
    let words = words(text);
    let mut collapsed: Option<String> = None;
    // How far the original has been carried over into `collapsed`.
    let mut copied = 0usize;

    let mut i = 0;
    while i < words.len() {
        let Some((period, repeats)) = longest_run(&words, i) else {
            i += 1;
            continue;
        };
        let Some(keep) = survivors(period, repeats) else {
            i += 1;
            continue;
        };

        // Everything between the end of the last surviving copy and the end of
        // the run goes, punctuation and spacing included — the comma after
        // "jetzt" belongs to the repetition, not to the sentence that is kept.
        let cut_from = words[i + keep * period - 1].end;
        let cut_to = words[i + repeats * period - 1].end;

        let buffer = collapsed.get_or_insert_with(|| String::with_capacity(text.len()));
        buffer.push_str(&text[copied..cut_from]);
        copied = cut_to;

        i += repeats * period;
    }

    match collapsed {
        Some(mut buffer) => {
            buffer.push_str(&text[copied..]);
            Cow::Owned(buffer.trim().to_string())
        }
        None => Cow::Borrowed(text),
    }
}

/// The longest consecutive repetition starting at `i`, as (period, repeats).
///
/// Longest by *coverage* — period times repeats — rather than by repeat count,
/// so that a four-word loop is recognised as one four-word loop instead of as
/// whatever shorter cycle happens to fall out of its first words. On a tie the
/// shorter period wins, which is the same decision seen from the other side.
fn longest_run(words: &[Word], i: usize) -> Option<(usize, usize)> {
    let mut best: Option<(usize, usize)> = None;

    for period in 1..=MAX_PERIOD.min(words.len() - i) {
        let mut repeats = 1;
        while i + period * (repeats + 1) <= words.len()
            && (0..period).all(|k| words[i + k].key == words[i + repeats * period + k].key)
        {
            repeats += 1;
        }
        if repeats < 2 {
            continue;
        }
        if best.is_none_or(|(p, r)| period * repeats > p * r) {
            best = Some((period, repeats));
        }
    }

    best
}

/// How many copies of a run survive, or `None` when it is not a loop at all.
///
/// The thresholds are the whole judgement call, so they are written out rather
/// than derived: a single word said five times is emphasis three times over
/// and a loop after that; a four-word phrase said three times in a row has
/// never once been somebody talking.
fn survivors(period: usize, repeats: usize) -> Option<usize> {
    let (threshold, keep) = match period {
        1 => (5, 3),
        2..=3 => (4, 2),
        _ => (3, 1),
    };
    (repeats >= threshold).then_some(keep)
}

/// Splits the text into words, keeping each one's span in the original.
///
/// A word is a run of alphanumerics plus the characters that sit *inside*
/// words rather than between them — apostrophes and hyphens, so that "geht's"
/// and "Front-of-House" compare as one token each.
fn words(text: &str) -> Vec<Word> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;

    for (i, ch) in text.char_indices() {
        if ch.is_alphanumeric() || matches!(ch, '\'' | '\u{2019}' | '-') {
            start.get_or_insert(i);
        } else if let Some(s) = start.take() {
            out.push(Word {
                end: i,
                key: text[s..i].to_lowercase(),
            });
        }
    }
    if let Some(s) = start {
        out.push(Word {
            end: text.len(),
            key: text[s..].to_lowercase(),
        });
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verbatim from a live run: the sentence before it is real speech and has
    /// to survive intact.
    #[test]
    fn the_loop_from_the_field_report_is_cut() {
        let text = "Gerade in der Message. Und das will ich ändern. Das will ich unbedingt \
                    ändern. Das ist schon jetzt, das ist schon jetzt, das ist schon jetzt, \
                    das ist schon jetzt, das ist schon jetzt, das ist schon jetzt, das ist \
                    schon jetzt, das ist schon jetzt, das ist schon jetzt, das ist schon jetzt";
        assert_eq!(
            collapse_repetitions(text),
            "Gerade in der Message. Und das will ich ändern. Das will ich unbedingt ändern. \
             Das ist schon jetzt"
        );
    }

    #[test]
    fn text_without_a_loop_is_returned_untouched_and_unallocated() {
        let text = "Standby für Cue fünf, dann Blackout.";
        assert!(matches!(collapse_repetitions(text), Cow::Borrowed(_)));
    }

    #[test]
    fn spoken_repetition_survives() {
        // All of these are things an operator actually says.
        for spoken in [
            "Go, go, go!",
            "Nein nein nein.",
            "Achtung, Achtung.",
            "Cue fünf, Cue fünf.",
            "Ja. Ja. Ja. Ja.",
        ] {
            assert_eq!(collapse_repetitions(spoken), spoken, "{spoken:?}");
        }
    }

    #[test]
    fn a_single_word_loop_keeps_three_copies() {
        assert_eq!(
            collapse_repetitions("Danke danke danke danke danke danke danke."),
            "Danke danke danke."
        );
    }

    #[test]
    fn a_short_phrase_loop_keeps_two_copies() {
        assert_eq!(
            collapse_repetitions("Vielen Dank. Vielen Dank. Vielen Dank. Vielen Dank."),
            "Vielen Dank. Vielen Dank."
        );
    }

    #[test]
    fn capitalisation_does_not_hide_a_repeat() {
        assert_eq!(
            collapse_repetitions("das ist schon jetzt, Das ist schon jetzt, das ist schon jetzt."),
            "das ist schon jetzt."
        );
    }

    #[test]
    fn the_longest_cycle_wins_over_the_shortest() {
        // Period two throughout; the cut must not land inside the phrase.
        let text = "ab cd ab cd ab cd ab cd";
        assert_eq!(collapse_repetitions(text), "ab cd ab cd");
    }

    #[test]
    fn what_follows_a_loop_is_kept() {
        assert_eq!(
            collapse_repetitions("Kanal frei, Kanal frei, Kanal frei, Kanal frei. Ende."),
            "Kanal frei, Kanal frei. Ende."
        );
    }

    #[test]
    fn two_loops_in_one_segment_are_both_cut() {
        let text = "eins zwei drei eins zwei drei eins zwei drei eins zwei drei und \
                    vier fünf sechs vier fünf sechs vier fünf sechs vier fünf sechs";
        assert_eq!(
            collapse_repetitions(text),
            "eins zwei drei eins zwei drei und vier fünf sechs vier fünf sechs"
        );
    }

    #[test]
    fn a_long_repeated_sentence_is_left_alone() {
        // Nine words: past MAX_PERIOD, where a repeat is a read-back rather
        // than a decoder artefact.
        let sentence = "Wir gehen jetzt auf Cue fünf und dann auf Blackout.";
        let text = [sentence, sentence, sentence].join(" ");
        assert_eq!(collapse_repetitions(&text), text);
    }

    #[test]
    fn empty_and_wordless_input_do_not_panic() {
        assert_eq!(collapse_repetitions(""), "");
        assert_eq!(collapse_repetitions("... ... ..."), "... ... ...");
    }
}
