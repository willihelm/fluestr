//! Alert terms: the words that must not be missed (P17).
//!
//! One global list, like the vocabulary and for the same reason: "Feuer",
//! "Arzt", the name of the production's own emergency cue mean the same thing
//! on every talkback, and a list per channel is eight chances for the one
//! channel that matters to be the one without it.
//!
//! The list is compiled once, when it changes, and every final transcript is
//! matched against it here — before the event crosses the bridge. The ranges
//! travel *with* the transcript, so the main chat, a detached group window and
//! the LAN viewer (P16) highlight exactly the same characters rather than each
//! running a matcher of its own that agrees with the others until the day a
//! term contains an umlaut.
//!
//! Terms and transcripts are normalised to NFC before they meet. "Bühne"
//! typed on a Mac keyboard and "Bu\u{308}hne" as the decoder emitted it are the
//! same word to the operator who wrote the list, and an alert list that
//! silently does not fire is worse than no alert list — it is the failure
//! nobody notices until the sentence that mattered has scrolled away.
//!
//! Matching is case-insensitive and on whole words only. Whole words because
//! the false positive is what destroys the feature: an operator who has been
//! shown three highlights that were the middle of another word stops looking
//! at the fourth. The cost is that a German compound does not match its parts —
//! "Feuer" does not fire on "Feuerwehrzufahrt" — which is the right way round
//! for a list an operator writes themselves: they can add the compound.

use serde::Serialize;
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

/// Where one alert term was found, as **UTF-16 code-unit** offsets into
/// [`crate::session::TranscriptEvent::text`].
///
/// UTF-16 and not bytes, and not `char`s: the only consumer is JavaScript,
/// whose strings are indexed in UTF-16 code units, and `"Bühne".slice(0, 2)`
/// has to mean what the backend meant. Bytes would cut an umlaut in half on
/// exactly the transcripts this app produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertMatch {
    pub start: usize,
    pub end: usize,
}

/// The compiled alert list.
///
/// Empty is the normal state — nobody has to use this feature — and an empty
/// matcher answers without touching the text at all, so a session that runs
/// without alert terms pays nothing per transcript.
#[derive(Debug, Clone, Default)]
pub struct Matcher {
    /// Normalised, lower-cased, trimmed, non-empty, without duplicates.
    /// Longest first, so that where two terms overlap the longer highlight is the one found
    /// first; the merge below makes the result independent of the order, but
    /// finding the long one first keeps the common case a single range.
    terms: Vec<String>,
}

impl Matcher {
    /// Compiles a list as the operator typed it.
    ///
    /// Blank entries are dropped rather than refused: the editor offers a row
    /// to type into, and a row that was added and not filled in must not turn
    /// every transcript into one long highlight.
    pub fn new<I, S>(terms: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut compiled: Vec<String> = Vec::new();
        for term in terms {
            let term: String = fold(term.as_ref().trim()).collect();
            if term.is_empty() || compiled.contains(&term) {
                continue;
            }
            compiled.push(term);
        }
        compiled.sort_by_key(|t| std::cmp::Reverse(t.chars().count()));

        Self { terms: compiled }
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    pub fn len(&self) -> usize {
        self.terms.len()
    }

    /// Every place a term occurs, in order and without overlaps.
    ///
    /// Overlapping hits are merged into one range: two terms that share a word
    /// are two reasons to highlight it and not two highlights, and a view that
    /// rendered them as nested spans would have to decide what the nesting
    /// looks like.
    pub fn find(&self, text: &str) -> Vec<AlertMatch> {
        if self.terms.is_empty() || text.is_empty() {
            return Vec::new();
        }

        // Normalising and lower-casing both change a string's length — "u" and
        // a combining diaeresis become one char, "İ" becomes two — so the
        // offsets of the folded copy are not the offsets of the text the UI
        // will render. `origin` maps every byte of the copy back to the byte in
        // `text` that produced it, with a final entry for the end so that a
        // match reaching the last character can be mapped too.
        //
        // The unit is a cluster and no longer a char, because composition is
        // what a starter and the marks after it do *together*: every byte a
        // cluster folds to maps back to where that cluster began, and a match
        // can only start or end where one does — the terms went through the
        // same fold, so nothing ever lands inside one.
        let mut lowered = String::with_capacity(text.len());
        let mut origin: Vec<usize> = Vec::with_capacity(text.len() + 1);
        let mut cluster = 0;
        for (at, ch) in text.char_indices() {
            if at > cluster && !is_combining_mark(ch) {
                fold_into(&text[cluster..at], cluster, &mut lowered, &mut origin);
                cluster = at;
            }
        }
        if cluster < text.len() {
            fold_into(&text[cluster..], cluster, &mut lowered, &mut origin);
        }
        origin.push(text.len());

        let mut found: Vec<(usize, usize)> = Vec::new();
        for term in &self.terms {
            for (at, hit) in lowered.match_indices(term.as_str()) {
                let end = at + hit.len();
                if is_word_bounded(&lowered, at, end) {
                    found.push((origin[at], origin[end]));
                }
            }
        }

        found.sort_unstable();

        let mut merged: Vec<AlertMatch> = Vec::new();
        for (start, end) in found {
            match merged.last_mut() {
                // Touching counts as overlapping: "Cue" and "Cue fünf" next to
                // each other are one thing to look at.
                Some(last) if start <= last.end => last.end = last.end.max(end),
                _ => merged.push(AlertMatch { start, end }),
            }
        }

        for range in &mut merged {
            range.start = utf16_len(&text[..range.start]);
            range.end = utf16_len(&text[..range.end]);
        }

        merged
    }
}

/// One term or one cluster of a transcript, as the matcher compares them:
/// composed, then lower-cased, then composed again — because lower-casing a
/// composed character can hand back a decomposed one.
fn fold(text: &str) -> impl Iterator<Item = char> + '_ {
    text.nfc().flat_map(char::to_lowercase).nfc()
}

/// Appends one folded cluster to `lowered`, recording `at` as the origin of
/// every byte it produced.
fn fold_into(cluster: &str, at: usize, lowered: &mut String, origin: &mut Vec<usize>) {
    lowered.extend(fold(cluster));
    while origin.len() < lowered.len() {
        origin.push(at);
    }
}

/// Whether the hit at `start..end` is a whole word rather than the inside of a
/// longer one.
///
/// "Word" is `char::is_alphanumeric`, which is what makes this work on German:
/// umlauts and ß are letters, so "Bühne" is one word and not three. A term
/// that begins or ends with punctuation ("112", "Code 3") is unaffected — the
/// check only ever looks at the characters *around* the hit.
fn is_word_bounded(text: &str, start: usize, end: usize) -> bool {
    let before = text[..start].chars().next_back();
    let after = text[end..].chars().next();
    !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
}

fn utf16_len(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The words are German on purpose: this is speech data, not code.
    fn matcher() -> Matcher {
        Matcher::new(["Feuer", "Arzt", "Cue fünf"])
    }

    fn found<'a>(matcher: &Matcher, text: &'a str) -> Vec<&'a str> {
        // Back through UTF-16, the way the UI will slice it — so this checks
        // the offsets and not only that something was found.
        let units: Vec<u16> = text.encode_utf16().collect();
        matcher
            .find(text)
            .iter()
            .map(|m| {
                let slice = String::from_utf16(&units[m.start..m.end]).expect("a whole slice");
                let at = text.find(&slice).expect("the slice is in the text");
                &text[at..at + slice.len()]
            })
            .collect()
    }

    #[test]
    fn an_empty_list_matches_nothing() {
        assert!(Matcher::default().find("Feuer im Saal").is_empty());
        assert!(Matcher::new(["", "   "]).is_empty());
    }

    #[test]
    fn a_term_matches_whatever_case_it_was_spoken_in() {
        assert_eq!(found(&matcher(), "FEUER im Saal"), ["FEUER"]);
        assert_eq!(found(&matcher(), "wir brauchen einen arzt"), ["arzt"]);
    }

    #[test]
    fn the_middle_of_a_longer_word_is_not_a_match() {
        // The false positive that would make an operator stop looking at the
        // highlights altogether.
        assert!(found(&matcher(), "Feuerwehrzufahrt freihalten").is_empty());
        assert!(found(&matcher(), "Arztpraxis").is_empty());
    }

    #[test]
    fn offsets_survive_umlauts() {
        // The reason the ranges are in UTF-16 units: everything before the
        // term here is longer in bytes than it is on screen.
        let text = "Bühne für Regie: Cue fünf";
        assert_eq!(found(&matcher(), text), ["Cue fünf"]);

        let hit = matcher().find(text)[0];
        let units: Vec<u16> = text.encode_utf16().collect();
        assert_eq!(
            String::from_utf16(&units[hit.start..hit.end]).unwrap(),
            "Cue fünf"
        );
    }

    /// "Bühne" as one character, and as "u" plus a combining diaeresis. The
    /// decoder emits one, a keyboard produces the other, and the operator
    /// cannot see which they typed.
    const COMPOSED: &str = "Bühne";
    const DECOMPOSED: &str = "Bu\u{308}hne";

    #[test]
    fn a_composed_term_matches_a_decomposed_transcript() {
        let matcher = Matcher::new([COMPOSED]);
        assert_eq!(
            found(&matcher, &format!("Achtung {DECOMPOSED} frei")),
            [DECOMPOSED]
        );
    }

    #[test]
    fn a_decomposed_term_matches_a_composed_transcript() {
        let matcher = Matcher::new([DECOMPOSED]);
        assert_eq!(
            found(&matcher, &format!("Achtung {COMPOSED} frei")),
            [COMPOSED]
        );
    }

    #[test]
    fn the_two_spellings_are_one_term() {
        assert_eq!(Matcher::new([COMPOSED, DECOMPOSED]).len(), 1);
    }

    #[test]
    fn a_decomposed_hit_is_still_a_whole_word() {
        // The mapping is per cluster, so a boundary check that fell between a
        // letter and its own accent would show up here as a false positive.
        let matcher = Matcher::new([COMPOSED]);
        assert!(found(&matcher, "Bu\u{308}hnenbild").is_empty());
    }

    #[test]
    fn a_decomposed_hit_reports_the_range_it_occupies_on_screen() {
        // Not the range the composed spelling would have had: the UI slices
        // the text that arrived, which is two code units longer here.
        let text = format!("Achtung {DECOMPOSED}");
        let hit = Matcher::new([COMPOSED]).find(&text)[0];
        assert_eq!(hit.start, "Achtung ".len());
        assert_eq!(hit.end, hit.start + DECOMPOSED.encode_utf16().count());
    }

    #[test]
    fn several_hits_come_back_in_order() {
        assert_eq!(
            found(&matcher(), "Feuer im Saal, sofort einen Arzt"),
            ["Feuer", "Arzt"]
        );
    }

    #[test]
    fn overlapping_terms_are_one_highlight() {
        let matcher = Matcher::new(["Cue", "Cue fünf"]);
        assert_eq!(found(&matcher, "jetzt Cue fünf"), ["Cue fünf"]);
    }

    #[test]
    fn the_same_term_twice_is_compiled_once() {
        assert_eq!(Matcher::new(["Feuer", " feuer "]).len(), 1);
    }

    #[test]
    fn punctuation_around_a_term_does_not_hide_it() {
        assert_eq!(found(&matcher(), "„Feuer!\" rief die Regie"), ["Feuer"]);
    }

    #[test]
    fn a_term_of_several_words_matches_across_the_space() {
        assert_eq!(found(&matcher(), "Achtung Cue fünf geht"), ["Cue fünf"]);
        assert!(found(&matcher(), "Cue sechs geht").is_empty());
    }
}
