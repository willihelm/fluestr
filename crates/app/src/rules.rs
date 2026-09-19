//! Rules: a trigger, and what it does (P19).
//!
//! This replaces the two hard-wired reactions the app had. An alert term used
//! to highlight its word *and* ask for attention because that is what an alert
//! term was; a view's "flash on every message" flooded the window because that
//! is what the checkbox was called. Neither could be narrowed to a channel,
//! neither could be combined, and "Feuer, but only on the stage manager's
//! channel" could not be written at all. A rule separates the two halves: a
//! trigger says *when*, an action says *what*, and the interesting cases are
//! the conjunctions.
//!
//! **A channel is not a group, and the two answer different questions.** A
//! channel is an *input*: naming one says where the words have to be said. A
//! group is a *view* — a window — and naming one says which window reacts.
//! They were one list once, which is the reason a rule narrowed to a group
//! still flooded every window that happened to show the channel: narrowing
//! the trigger cannot narrow the audience. So a rule has both, separately: a
//! trigger of words and streams, and a flash that names the views it is for.
//!
//! **A rule arrives compiled.** The frontend owns the groups — the backend has
//! never had a notion of one — so a channel trigger is resolved into stream
//! ids before it gets here, and a group is reduced to the one thing this side
//! can carry: its *position* in the active profile, which is also how a group
//! window is addressed (`group-3`). The backend never asks what a view means;
//! it hands the positions back with the event, and each window compares them
//! with the view it is. See [`CompiledRule`].
//!
//! **Where it runs.** Here, in the one place every view's copy of a transcript
//! comes from ([`crate::TauriEvents::transcript`]), and not in each window.
//! The visible actions could in principle be decided per view; the ones P19
//! adds later — a MIDI cue, a webhook — cannot, because they must fire exactly
//! once per event and not once per open window, nor zero times because every
//! window was closed. Deciding all of them in one place is what keeps those
//! two kinds of action on one list rather than in two features that drift.
//! Each view then applies the *visible* actions by its own scope: a group
//! window does not flood for a channel it is not showing, and no window
//! floods for a rule addressed to another view.
//!
//! **Rate limiting is per rule**, because a channel trigger fires on every
//! bubble and a busy talkback is several per minute. Zero — the default — is
//! unthrottled, which is the right default for a rule that only highlights.
//! A rule inside its interval does nothing at all, including highlighting:
//! "this rule is quiet for a second" is one state an operator can hold, and a
//! rule that half-fires is two.

use std::collections::{BTreeSet, HashSet};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::alerts::{AlertMatch, Matcher};

/// What a rule does when it fires.
///
/// Only the three visible ones exist so far. The MIDI cue and the webhook P19
/// describes are the reason the evaluation is here rather than in each window,
/// and they are arms added to this enum rather than a second mechanism beside
/// it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "rules.ts")]
pub enum Action {
    /// Floods the window in dark orange for one pulse — the loud request, and
    /// the one made even to a window that is already focused. What a view's
    /// "flash on every new message" used to be.
    Flash,
    /// Marks the matched words in the chat and rings the bubble holding them.
    /// The only action a rule without a word trigger cannot ask for: there is
    /// nothing to mark.
    Highlight,
    /// Asks the platform for attention once — a single bounce of the Dock
    /// icon, a single pulse of a browser tab.
    Attention,
    /// Asks and keeps asking until the app is looked at. What an alert term
    /// did, and still the right answer for a word that must not be missed.
    UrgentAttention,
}

/// The order actions are reported in, so that two rules firing on one segment
/// produce one list and not two orders of the same list.
const ORDER: [Action; 4] = [
    Action::Highlight,
    Action::Flash,
    Action::Attention,
    Action::UrgentAttention,
];

/// One rule as the frontend compiled it.
///
/// The words arrive as the operator typed them and are compiled here, by the
/// same matcher the alert list always used: case-insensitive, NFC-normalised,
/// whole words. An empty list is *no word condition* — "anything at all on
/// channel 3" — which is exactly not what an empty [`Matcher`] means, so the
/// two are kept apart in [`Rule::words`].
#[derive(Debug, Clone, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "rules.ts")]
pub struct CompiledRule {
    /// Only ever logged. A rule is addressed by nothing — it is evaluated,
    /// not referred to — but a rule that fires unexpectedly has to be findable
    /// on the Settings page, and the operator named it there.
    pub name: String,
    #[serde(default)]
    pub words: Vec<String>,
    /// The streams this rule listens to, or `None` for every stream.
    ///
    /// `None` and not an empty list: a rule whose channels have all been
    /// deleted listens to *nothing*, and reading that as "everything" would
    /// turn a rule the operator narrowed into the loudest one they have. The
    /// frontend drops such a rule instead of sending an empty list, and this
    /// side still cannot confuse the two.
    pub streams: Option<Vec<u16>>,
    /// Which windows the *flash* is for: group positions in the active
    /// profile, or `None` for every window showing the channel.
    ///
    /// Positions and not names, because that is what a group window is
    /// addressed by (`group-3`) and what the configuration file stores a
    /// group by; the keys the editor works in mean nothing outside the window
    /// that issued them. Only the flash is addressed this way — the other
    /// actions are about the session or the app, not about a view.
    #[serde(default)]
    pub views: Option<Vec<u16>>,
    pub actions: Vec<Action>,
    /// At most one firing per this many milliseconds. Zero is unthrottled.
    #[serde(default)]
    pub min_interval_ms: u64,
}

/// One compiled rule, with the clock it is rate-limited by.
#[derive(Debug)]
struct Rule {
    name: String,
    /// `None` is "no word condition". `Some` is always a non-empty matcher.
    words: Option<Matcher>,
    /// `None` is "every stream".
    streams: Option<HashSet<u16>>,
    /// `None` is "every window that shows the channel".
    views: Option<Vec<u16>>,
    actions: Vec<Action>,
    min_interval: Duration,
    /// When this rule last fired, for the rate limit. Per rule and not per
    /// action: one interval per rule is one number on the Settings page, and
    /// a rule whose flash was throttled while its highlight was not would be
    /// two rules wearing one name.
    fired: Option<Instant>,
}

/// What one transcript asked for.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Where to mark the text, in UTF-16 code units — merged across every rule
    /// that asked, so two rules matching the same word are one highlight.
    pub alerts: Vec<AlertMatch>,
    /// Every action asked for, each once, in [`ORDER`].
    pub actions: Vec<Action>,
    /// Which windows the flash is for, merged across every rule that asked:
    /// the union of their views, or `None` for every window showing the
    /// channel.
    ///
    /// `None` wins over any list, because the question each window asks is
    /// "was any flash meant for me" — one rule addressed to everything is
    /// already a yes. Only read when [`Outcome::actions`] holds
    /// [`Action::Flash`].
    pub flash_views: Option<Vec<u16>>,
}

impl Outcome {
    pub fn is_empty(&self) -> bool {
        self.alerts.is_empty() && self.actions.is_empty()
    }
}

/// The compiled rule list.
///
/// Empty is the normal state — nobody has to use this — and an empty list
/// answers without touching the text, so a session with no rules pays nothing
/// per transcript.
#[derive(Debug, Default)]
pub struct Rules {
    rules: Vec<Rule>,
}

impl Rules {
    /// Compiles what the frontend sent.
    ///
    /// A rule with no actions is dropped: it is a rule being written, and the
    /// editor offers an empty one to fill in. A rule with no words *and* no
    /// streams is kept — that is "every message", which is what the
    /// all-channels flash setting became.
    pub fn compile<I: IntoIterator<Item = CompiledRule>>(rules: I) -> Self {
        let rules = rules
            .into_iter()
            .filter(|rule| !rule.actions.is_empty())
            .map(|rule| {
                let words = match Matcher::new(&rule.words) {
                    matcher if matcher.is_empty() => None,
                    matcher => Some(matcher),
                };
                let mut actions = Vec::new();
                for action in ORDER {
                    if rule.actions.contains(&action) {
                        actions.push(action);
                    }
                }
                Rule {
                    name: rule.name,
                    words,
                    streams: rule.streams.map(HashSet::from_iter),
                    views: rule.views,
                    actions,
                    min_interval: Duration::from_millis(rule.min_interval_ms),
                    fired: None,
                }
            })
            .collect();

        Self { rules }
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    pub fn len(&self) -> usize {
        self.rules.len()
    }

    /// Runs every rule against one finished transcript.
    ///
    /// `now` is passed in rather than read here so that the rate limit is
    /// testable without sleeping through it.
    pub fn evaluate(&mut self, stream: u16, text: &str, now: Instant) -> Outcome {
        let mut alerts: Vec<AlertMatch> = Vec::new();
        let mut asked: HashSet<Action> = HashSet::new();
        // The views every flashing rule named, and whether one of them named
        // none — which is "every window" and settles the question on its own.
        let mut views: BTreeSet<u16> = BTreeSet::new();
        let mut every_window = false;

        for rule in &mut self.rules {
            if let Some(streams) = &rule.streams {
                if !streams.contains(&stream) {
                    continue;
                }
            }

            // Matched before the rate limit is consulted, and the limit before
            // it is consumed: a rule that did not match must not spend its
            // interval, or a busy channel would keep a rule quiet for words
            // that were never said.
            let hits = match &rule.words {
                Some(matcher) => {
                    let hits = matcher.find(text);
                    if hits.is_empty() {
                        continue;
                    }
                    hits
                }
                None => Vec::new(),
            };

            if let Some(last) = rule.fired {
                if now.saturating_duration_since(last) < rule.min_interval {
                    tracing::trace!(rule = %rule.name, "rate-limited");
                    continue;
                }
            }
            rule.fired = Some(now);

            if rule.actions.contains(&Action::Highlight) {
                alerts.extend(hits);
            }
            asked.extend(rule.actions.iter().copied());
            if rule.actions.contains(&Action::Flash) {
                match &rule.views {
                    Some(named) => views.extend(named.iter().copied()),
                    None => every_window = true,
                }
            }
        }

        Outcome {
            alerts: merge(alerts),
            actions: ORDER.into_iter().filter(|a| asked.contains(a)).collect(),
            flash_views: if every_window {
                None
            } else {
                Some(views.into_iter().collect())
            },
        }
    }
}

/// Overlapping ranges from different rules, as one set of highlights.
///
/// [`Matcher::find`] already does this within one rule; two rules whose words
/// share a hit are two reasons to mark it and not two marks, and a view that
/// rendered them as nested spans would have to decide what the nesting looks
/// like.
fn merge(mut ranges: Vec<AlertMatch>) -> Vec<AlertMatch> {
    ranges.sort_unstable_by_key(|r| (r.start, r.end));

    let mut merged: Vec<AlertMatch> = Vec::with_capacity(ranges.len());
    for range in ranges {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(
        name: &str,
        words: &[&str],
        streams: Option<&[u16]>,
        actions: &[Action],
    ) -> CompiledRule {
        CompiledRule {
            name: name.to_string(),
            words: words.iter().map(|w| w.to_string()).collect(),
            streams: streams.map(|s| s.to_vec()),
            views: None,
            actions: actions.to_vec(),
            min_interval_ms: 0,
        }
    }

    /// The same rule, addressed to the windows of those groups.
    fn for_views(mut rule: CompiledRule, views: &[u16]) -> CompiledRule {
        rule.views = Some(views.to_vec());
        rule
    }

    #[test]
    fn no_rules_ask_for_nothing() {
        let mut rules = Rules::default();
        assert!(rules
            .evaluate(0, "Feuer im Saal", Instant::now())
            .is_empty());
    }

    /// The case that could not be written before P19 at all.
    #[test]
    fn a_word_and_a_channel_is_a_conjunction() {
        let mut rules = Rules::compile([rule(
            "Feuer bei der Inspizienz",
            &["Feuer"],
            Some(&[2]),
            &[Action::Highlight, Action::UrgentAttention],
        )]);
        let now = Instant::now();

        // The word, on the wrong channel.
        assert!(rules.evaluate(1, "Feuer im Saal", now).is_empty());
        // The right channel, without the word.
        assert!(rules.evaluate(2, "Vorhang auf", now).is_empty());

        let fired = rules.evaluate(2, "Feuer im Saal", now);
        assert_eq!(fired.actions, [Action::Highlight, Action::UrgentAttention]);
        assert_eq!(fired.alerts, [AlertMatch { start: 0, end: 5 }]);
    }

    /// "Anything at all on channel 3" — the second rule P19 exists for.
    #[test]
    fn a_rule_without_words_fires_on_every_message_of_its_channel() {
        let mut rules = Rules::compile([rule("Kanal 3", &[], Some(&[3]), &[Action::Flash])]);
        let now = Instant::now();

        assert_eq!(
            rules.evaluate(3, "Vorhang auf", now).actions,
            [Action::Flash]
        );
        assert!(rules.evaluate(4, "Vorhang auf", now).is_empty());
    }

    /// What the all-channels view's flash setting became.
    #[test]
    fn a_rule_with_neither_words_nor_streams_fires_on_everything() {
        let mut rules = Rules::compile([rule("Alles", &[], None, &[Action::Flash])]);
        let now = Instant::now();

        assert_eq!(rules.evaluate(0, "irgendwas", now).actions, [Action::Flash]);
        assert_eq!(rules.evaluate(7, "", now).actions, [Action::Flash]);
    }

    /// A rule that only asks for attention contributes no highlight, however
    /// well its words matched: the ranges are what the *Highlight* action is.
    #[test]
    fn only_a_highlighting_rule_marks_the_text() {
        let mut rules = Rules::compile([rule("Arzt", &["Arzt"], None, &[Action::Attention])]);
        let fired = rules.evaluate(0, "wir brauchen einen Arzt", Instant::now());
        assert_eq!(fired.actions, [Action::Attention]);
        assert!(fired.alerts.is_empty());
    }

    #[test]
    fn two_rules_on_one_segment_ask_once_each() {
        let mut rules = Rules::compile([
            rule("a", &["Feuer"], None, &[Action::Highlight, Action::Flash]),
            rule(
                "b",
                &["Feuer"],
                None,
                &[Action::Flash, Action::UrgentAttention],
            ),
        ]);
        let fired = rules.evaluate(0, "Feuer", Instant::now());
        assert_eq!(
            fired.actions,
            [Action::Highlight, Action::Flash, Action::UrgentAttention]
        );
    }

    #[test]
    fn overlapping_highlights_from_two_rules_are_one_mark() {
        let mut rules = Rules::compile([
            rule("kurz", &["Cue"], None, &[Action::Highlight]),
            rule("lang", &["Cue fünf"], None, &[Action::Highlight]),
        ]);
        let fired = rules.evaluate(0, "jetzt Cue fünf", Instant::now());
        assert_eq!(fired.alerts, [AlertMatch { start: 6, end: 14 }]);
    }

    #[test]
    fn a_rate_limited_rule_stays_quiet_until_its_interval_is_up() {
        let mut compiled = rule("Kanal 3", &[], Some(&[3]), &[Action::Flash]);
        compiled.min_interval_ms = 1_000;
        let mut rules = Rules::compile([compiled]);

        let start = Instant::now();
        assert_eq!(rules.evaluate(3, "eins", start).actions, [Action::Flash]);
        assert!(rules
            .evaluate(3, "zwei", start + Duration::from_millis(999))
            .is_empty());
        assert_eq!(
            rules
                .evaluate(3, "drei", start + Duration::from_millis(1_000))
                .actions,
            [Action::Flash]
        );
    }

    /// The limit is spent by firing, not by being asked. Otherwise a channel
    /// nobody stops talking on would keep a word rule quiet for a word that
    /// was never said.
    #[test]
    fn a_rule_that_did_not_match_does_not_spend_its_interval() {
        let mut compiled = rule("Feuer", &["Feuer"], None, &[Action::Flash]);
        compiled.min_interval_ms = 10_000;
        let mut rules = Rules::compile([compiled]);

        let start = Instant::now();
        for i in 0..5 {
            assert!(rules
                .evaluate(0, "nichts besonderes", start + Duration::from_millis(i))
                .is_empty());
        }
        assert_eq!(
            rules
                .evaluate(0, "Feuer", start + Duration::from_millis(5))
                .actions,
            [Action::Flash]
        );
    }

    /// A rule being written is not a rule that fires on everything.
    #[test]
    fn a_rule_without_actions_is_dropped() {
        let rules = Rules::compile([rule("halb fertig", &[], None, &[])]);
        assert!(rules.is_empty());
    }

    /// A flash addressed to one view says so, and says nothing about the
    /// others: the whole point of naming a group is that the windows which do
    /// not answer for it stay quiet.
    #[test]
    fn a_flash_carries_the_views_it_was_addressed_to() {
        let mut rules = Rules::compile([for_views(
            rule("Test auf FOH", &["Test"], Some(&[0]), &[Action::Flash]),
            &[1, 0],
        )]);

        let fired = rules.evaluate(0, "Test", Instant::now());
        assert_eq!(fired.actions, [Action::Flash]);
        // Sorted and deduplicated, so two lists meaning the same thing are
        // one list.
        assert_eq!(fired.flash_views.as_deref(), Some(&[0u16, 1][..]));
    }

    /// An unaddressed flash is the old behaviour and has to stay reachable:
    /// every window showing the channel.
    #[test]
    fn a_flash_without_views_is_for_every_window() {
        let mut rules = Rules::compile([rule("Alles", &[], None, &[Action::Flash])]);
        assert!(rules
            .evaluate(3, "irgendwas", Instant::now())
            .flash_views
            .is_none());
    }

    /// Two flashing rules on one segment are one question per window — "was
    /// any of this meant for me" — so an unaddressed one settles it.
    #[test]
    fn an_unaddressed_flash_outranks_an_addressed_one() {
        let mut rules = Rules::compile([
            for_views(rule("eng", &["Test"], None, &[Action::Flash]), &[2]),
            rule("weit", &["Test"], None, &[Action::Flash]),
        ]);
        assert!(rules
            .evaluate(0, "Test", Instant::now())
            .flash_views
            .is_none());
    }

    /// Only the flash is addressed to a view. An attention request is the
    /// app's, not a window's, and a rule that asks for one without flashing
    /// says nothing about windows at all.
    #[test]
    fn views_are_not_reported_for_a_rule_that_does_not_flash() {
        let mut rules = Rules::compile([for_views(
            rule("Arzt", &["Arzt"], None, &[Action::UrgentAttention]),
            &[1],
        )]);
        let fired = rules.evaluate(0, "Arzt", Instant::now());
        assert_eq!(fired.actions, [Action::UrgentAttention]);
        assert_eq!(fired.flash_views.as_deref(), Some(&[][..]));
    }

    /// The wire shape the frontend sends. Pinned because a rule is compiled on
    /// one side and evaluated on the other, and a field misspelled in the
    /// middle is a rule that silently stops narrowing.
    #[test]
    fn a_compiled_rule_arrives_as_a_frontend_writes_it() {
        let rule: CompiledRule = serde_json::from_str(
            r#"{"name":"Go","words":["Go"],"streams":[2],"views":[1],"actions":["highlight","urgentAttention"],"minIntervalMs":500}"#,
        )
        .expect("a compiled rule");

        assert_eq!(rule.streams.as_deref(), Some(&[2u16][..]));
        assert_eq!(rule.views.as_deref(), Some(&[1u16][..]));
        assert_eq!(rule.actions, [Action::Highlight, Action::UrgentAttention]);
        assert_eq!(rule.min_interval_ms, 500);
    }
}
