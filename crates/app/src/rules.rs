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
//! The visible actions could in principle be decided per view; the outbound
//! one — the webhook — cannot, because it must fire exactly once per event and
//! not once per open window, nor zero times because every window was closed.
//! Deciding all of them in one place is what keeps the two kinds of action on
//! one list rather than in two features that drift. Each view then applies the
//! *visible* actions by its own scope: a group window does not flood for a
//! channel it is not showing, and no window floods for a rule addressed to
//! another view.
//!
//! **The outbound action leaves by another door.** A webhook does not travel
//! with the event: it goes into [`Outcome::webhooks`], already rendered, and
//! from there onto the sending queue ([`crate::webhook`]). Putting it in the
//! reported actions would send every browser the LAN remote serves a note that
//! a rule it is deliberately not shown the destination of has just fired — and
//! would invite a view to act on it, which is the one thing that must happen
//! exactly once.
//!
//! **Rate limiting is per rule**, because a channel trigger fires on every
//! bubble and a busy talkback is several per minute. Zero — the default — is
//! unthrottled, which is the right default for a rule that only highlights.
//! A rule inside its interval does nothing at all, including highlighting:
//! "this rule is quiet for a second" is one state an operator can hold, and a
//! rule that half-fires is two.

use std::collections::{BTreeSet, HashSet};
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};

use crate::alerts::{AlertMatch, Matcher};
use crate::webhook::{self, Webhook};

/// What a rule does when it fires.
///
/// Four of the five are visible — they happen in a window, and every window
/// decides for itself whether it is the one. The fifth, [`Action::Webhook`],
/// is the reason the evaluation is here rather than in each window: it leaves
/// the building, so it has to leave exactly once.
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
    /// Calls the rule's [`Webhook`] — the action the app is wanted for in a
    /// theatre (P19). The only one that leaves the machine, the only one that
    /// is *not* reported back to the views, and the only one a rule carries a
    /// payload for: the request itself is [`CompiledRule::webhook`], because
    /// it qualifies exactly this action, the way [`CompiledRule::views`]
    /// qualifies the flash.
    Webhook,
}

/// The order the **visible** actions are reported in, so that two rules firing
/// on one segment produce one list and not two orders of the same list.
///
/// [`Action::Webhook`] is deliberately absent: it is not reported to a view at
/// all, and a list that held it would be a list every window had to remember
/// to ignore. See [`Outcome::webhooks`].
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
    /// The request [`Action::Webhook`] makes, or `None` for a rule that does
    /// not make one.
    ///
    /// Beside the actions rather than inside them, exactly like
    /// [`CompiledRule::views`]: it qualifies one action, and putting it in the
    /// enum would put a URL into the list that travels back out with every
    /// transcript. The two halves are married in [`Rules::compile`], which
    /// keeps a webhook only for a rule that asked for one.
    #[serde(default)]
    pub webhook: Option<Webhook>,
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
    /// `Some` only for a rule that asked for [`Action::Webhook`] *and* wrote
    /// one. A rule that ticked the box and has not typed a URL yet is a rule
    /// being written.
    webhook: Option<Webhook>,
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
    /// The requests to make, already rendered against this segment.
    ///
    /// Not part of [`Outcome::actions`] and never sent to a view: an outbound
    /// action is the session's, not a window's. One entry per *rule* that
    /// fired, in rule order, because two rules pointing at the same endpoint
    /// are two cues and not one.
    pub webhooks: Vec<webhook::Request>,
}

impl Outcome {
    pub fn is_empty(&self) -> bool {
        self.alerts.is_empty() && self.actions.is_empty() && self.webhooks.is_empty()
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
    /// A rule with nothing to do is dropped: it is a rule being written, and
    /// the editor offers an empty one to fill in. A rule with no words *and*
    /// no streams is kept — that is "every message", which is what the
    /// all-channels flash setting became.
    ///
    /// "Nothing to do" is asked after the two halves are married, not before:
    /// a rule whose only action is the webhook has an empty visible list and
    /// is perfectly real, and a rule that ticked the webhook box without
    /// typing a URL has a full one and is not.
    pub fn compile<I: IntoIterator<Item = CompiledRule>>(rules: I) -> Self {
        let rules = rules
            .into_iter()
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
                // The request is kept only for a rule that asked to make one.
                // A webhook left behind on a rule whose box was un-ticked is a
                // destination the operator believes they switched off.
                let webhook = rule
                    .actions
                    .contains(&Action::Webhook)
                    .then_some(rule.webhook)
                    .flatten()
                    .filter(|hook| !hook.url.trim().is_empty());

                Rule {
                    name: rule.name,
                    words,
                    streams: rule.streams.map(HashSet::from_iter),
                    views: rule.views,
                    actions,
                    webhook,
                    min_interval: Duration::from_millis(rule.min_interval_ms),
                    fired: None,
                }
            })
            .filter(|rule| !rule.actions.is_empty() || rule.webhook.is_some())
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
    /// testable without sleeping through it. The wall clock a `{{timestamp}}`
    /// is rendered from is read separately, and only if a webhook actually
    /// fires: an [`Instant`] cannot be formatted as a date, and nothing else
    /// here needs one.
    pub fn evaluate(&mut self, segment: Segment<'_>, now: Instant) -> Outcome {
        let Segment {
            stream,
            label,
            text,
        } = segment;

        let mut alerts: Vec<AlertMatch> = Vec::new();
        let mut asked: HashSet<Action> = HashSet::new();
        // The views every flashing rule named, and whether one of them named
        // none — which is "every window" and settles the question on its own.
        let mut views: BTreeSet<u16> = BTreeSet::new();
        let mut every_window = false;
        let mut webhooks: Vec<webhook::Request> = Vec::new();
        // Read at most once per segment, and only for the first rule that
        // needs it, so that two rules firing on one sentence stamp the same
        // instant — they are one event.
        let mut at: Option<String> = None;

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

            if let Some(hook) = &rule.webhook {
                let timestamp = at
                    .get_or_insert_with(|| webhook::timestamp(SystemTime::now()))
                    .clone();
                webhooks.push(hook.render(&webhook::Values {
                    text: text.to_string(),
                    channel: label.to_string(),
                    rule: rule.name.clone(),
                    matched: matched(text, &hits),
                    timestamp,
                }));
            }

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
            webhooks,
        }
    }
}

/// One finished transcript, as the rules see it.
///
/// A struct rather than three arguments because the label is here only for the
/// webhook's `{{channel}}`, and a bare `&str` in third place next to another
/// `&str` is exactly the pair that gets swapped at a call site.
#[derive(Debug, Clone, Copy)]
pub struct Segment<'a> {
    /// Which input it was said on. What a trigger narrows by.
    pub stream: u16,
    /// What the operator calls that input. Never matched against — it is the
    /// readable half of the stream id, for a request that has to name a
    /// channel to somebody outside this process.
    pub label: &'a str,
    pub text: &'a str,
}

/// The words that actually matched, joined by a space.
///
/// The ranges are UTF-16 code units, because that is what the only other
/// consumer indexes in (see [`AlertMatch`]), so this walks the string rather
/// than slicing it: a byte index would cut an umlaut in half on exactly the
/// transcripts this app produces.
fn matched(text: &str, hits: &[AlertMatch]) -> String {
    if hits.is_empty() {
        return String::new();
    }

    let mut pieces = vec![String::new(); hits.len()];
    let mut at = 0usize;
    for ch in text.chars() {
        let next = at + ch.len_utf16();
        for (piece, hit) in pieces.iter_mut().zip(hits) {
            if at >= hit.start && next <= hit.end {
                piece.push(ch);
            }
        }
        at = next;
    }
    pieces.join(" ")
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
            webhook: None,
            min_interval_ms: 0,
        }
    }

    /// A segment on `stream`, said on a channel nobody named. The label only
    /// matters where a webhook renders `{{channel}}`, so the tests that are
    /// not about one say so by leaving it blank.
    fn said(stream: u16, text: &str) -> Segment<'_> {
        Segment {
            stream,
            label: "",
            text,
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
            .evaluate(said(0, "Feuer im Saal"), Instant::now())
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
        assert!(rules.evaluate(said(1, "Feuer im Saal"), now).is_empty());
        // The right channel, without the word.
        assert!(rules.evaluate(said(2, "Vorhang auf"), now).is_empty());

        let fired = rules.evaluate(said(2, "Feuer im Saal"), now);
        assert_eq!(fired.actions, [Action::Highlight, Action::UrgentAttention]);
        assert_eq!(fired.alerts, [AlertMatch { start: 0, end: 5 }]);
    }

    /// "Anything at all on channel 3" — the second rule P19 exists for.
    #[test]
    fn a_rule_without_words_fires_on_every_message_of_its_channel() {
        let mut rules = Rules::compile([rule("Kanal 3", &[], Some(&[3]), &[Action::Flash])]);
        let now = Instant::now();

        assert_eq!(
            rules.evaluate(said(3, "Vorhang auf"), now).actions,
            [Action::Flash]
        );
        assert!(rules.evaluate(said(4, "Vorhang auf"), now).is_empty());
    }

    /// What the all-channels view's flash setting became.
    #[test]
    fn a_rule_with_neither_words_nor_streams_fires_on_everything() {
        let mut rules = Rules::compile([rule("Alles", &[], None, &[Action::Flash])]);
        let now = Instant::now();

        assert_eq!(
            rules.evaluate(said(0, "irgendwas"), now).actions,
            [Action::Flash]
        );
        assert_eq!(rules.evaluate(said(7, ""), now).actions, [Action::Flash]);
    }

    /// A rule that only asks for attention contributes no highlight, however
    /// well its words matched: the ranges are what the *Highlight* action is.
    #[test]
    fn only_a_highlighting_rule_marks_the_text() {
        let mut rules = Rules::compile([rule("Arzt", &["Arzt"], None, &[Action::Attention])]);
        let fired = rules.evaluate(said(0, "wir brauchen einen Arzt"), Instant::now());
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
        let fired = rules.evaluate(said(0, "Feuer"), Instant::now());
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
        let fired = rules.evaluate(said(0, "jetzt Cue fünf"), Instant::now());
        assert_eq!(fired.alerts, [AlertMatch { start: 6, end: 14 }]);
    }

    #[test]
    fn a_rate_limited_rule_stays_quiet_until_its_interval_is_up() {
        let mut compiled = rule("Kanal 3", &[], Some(&[3]), &[Action::Flash]);
        compiled.min_interval_ms = 1_000;
        let mut rules = Rules::compile([compiled]);

        let start = Instant::now();
        assert_eq!(
            rules.evaluate(said(3, "eins"), start).actions,
            [Action::Flash]
        );
        assert!(rules
            .evaluate(said(3, "zwei"), start + Duration::from_millis(999))
            .is_empty());
        assert_eq!(
            rules
                .evaluate(said(3, "drei"), start + Duration::from_millis(1_000))
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
                .evaluate(
                    said(0, "nichts besonderes"),
                    start + Duration::from_millis(i)
                )
                .is_empty());
        }
        assert_eq!(
            rules
                .evaluate(said(0, "Feuer"), start + Duration::from_millis(5))
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

        let fired = rules.evaluate(said(0, "Test"), Instant::now());
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
            .evaluate(said(3, "irgendwas"), Instant::now())
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
            .evaluate(said(0, "Test"), Instant::now())
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
        let fired = rules.evaluate(said(0, "Arzt"), Instant::now());
        assert_eq!(fired.actions, [Action::UrgentAttention]);
        assert_eq!(fired.flash_views.as_deref(), Some(&[][..]));
    }

    // --- The outbound action (P19) ---------------------------------------

    fn hook(url: &str, body: &str) -> Webhook {
        Webhook {
            url: url.to_string(),
            method: crate::webhook::Method::Post,
            headers: vec![],
            body: body.to_string(),
        }
    }

    /// A cue is a rule whose *only* action leaves the building. It has nothing
    /// to say to any window, and it is still a rule.
    #[test]
    fn a_rule_that_only_calls_out_is_kept_and_reports_nothing_visible() {
        let mut compiled = rule("Go", &["Go"], Some(&[2]), &[Action::Webhook]);
        compiled.webhook = Some(hook("http://desk:8000/api/location/1/0/2/press", ""));
        let mut rules = Rules::compile([compiled]);

        let fired = rules.evaluate(said(2, "Go für Cue 5"), Instant::now());
        assert!(fired.actions.is_empty());
        assert!(fired.alerts.is_empty());
        assert!(!fired.is_empty());
        assert_eq!(fired.webhooks.len(), 1);
        assert_eq!(
            fired.webhooks[0].url,
            "http://desk:8000/api/location/1/0/2/press"
        );
    }

    /// The webhook never travels with the event. Every browser the remote
    /// serves would otherwise be told that a rule it is not shown the
    /// destination of has just fired — and invited to act on it.
    #[test]
    fn the_outbound_action_is_never_reported_to_a_view() {
        let mut compiled = rule("Go", &["Go"], None, &[Action::Webhook, Action::Highlight]);
        compiled.webhook = Some(hook("http://desk/press", ""));
        let mut rules = Rules::compile([compiled]);

        let fired = rules.evaluate(said(0, "Go"), Instant::now());
        assert_eq!(fired.actions, [Action::Highlight]);
        assert!(!fired.actions.contains(&Action::Webhook));
    }

    /// The transcript on a button face, and the channel it was said on: the
    /// two values a MIDI note could never have carried.
    #[test]
    fn the_request_is_rendered_against_the_segment() {
        let mut compiled = rule("Letzter Ruf", &["Go"], None, &[Action::Webhook]);
        compiled.webhook = Some(hook(
            "http://desk:8000/api/custom-variable/last_call/value",
            "{{channel}}: {{matched}} — {{rule}}",
        ));
        let mut rules = Rules::compile([compiled]);

        let fired = rules.evaluate(
            Segment {
                stream: 0,
                label: "Inspizienz",
                text: "Go für Cue 5",
            },
            Instant::now(),
        );
        assert_eq!(fired.webhooks[0].body, "Inspizienz: Go — Letzter Ruf");
    }

    /// Un-ticking the box switches the destination off. A request left behind
    /// on a rule whose box is clear is a destination the operator believes
    /// they stopped calling.
    #[test]
    fn a_request_without_the_action_is_not_made() {
        let mut compiled = rule("Go", &["Go"], None, &[Action::Highlight]);
        compiled.webhook = Some(hook("http://desk/press", ""));
        let mut rules = Rules::compile([compiled]);

        assert!(rules
            .evaluate(said(0, "Go"), Instant::now())
            .webhooks
            .is_empty());
    }

    /// The box ticked and no URL typed yet is a rule being written, which is
    /// the same thing a rule with no actions is.
    #[test]
    fn the_action_without_a_url_is_a_rule_being_written() {
        let mut compiled = rule("Go", &["Go"], None, &[Action::Webhook]);
        compiled.webhook = Some(hook("   ", "{{text}}"));
        assert!(Rules::compile([compiled]).is_empty());

        let bare = rule("Go", &["Go"], None, &[Action::Webhook]);
        assert!(Rules::compile([bare]).is_empty());
    }

    /// The rate limit is what keeps a channel trigger on a busy talkback from
    /// becoming a flood of requests, and it applies to this action like any
    /// other — a rule inside its interval does nothing at all.
    #[test]
    fn the_rate_limit_holds_the_outbound_action_too() {
        let mut compiled = rule("Kanal 3", &[], Some(&[3]), &[Action::Webhook]);
        compiled.webhook = Some(hook("http://desk/press", ""));
        compiled.min_interval_ms = 1_000;
        let mut rules = Rules::compile([compiled]);

        let start = Instant::now();
        assert_eq!(rules.evaluate(said(3, "eins"), start).webhooks.len(), 1);
        assert!(rules
            .evaluate(said(3, "zwei"), start + Duration::from_millis(999))
            .webhooks
            .is_empty());
        assert_eq!(
            rules
                .evaluate(said(3, "drei"), start + Duration::from_secs(1))
                .webhooks
                .len(),
            1
        );
    }

    /// Two rules pointing at one endpoint are two cues. Unlike a highlight,
    /// which two rules agreeing on merge into one mark, an outbound request
    /// is an instruction and merging two of them would drop one.
    #[test]
    fn two_rules_calling_out_make_two_requests() {
        let mut first = rule("Licht", &["Go"], None, &[Action::Webhook]);
        first.webhook = Some(hook("http://desk/press/1", ""));
        let mut second = rule("Ton", &["Go"], None, &[Action::Webhook]);
        second.webhook = Some(hook("http://desk/press/2", ""));
        let mut rules = Rules::compile([first, second]);

        let fired = rules.evaluate(said(0, "Go"), Instant::now());
        let urls: Vec<&str> = fired.webhooks.iter().map(|r| r.url.as_str()).collect();
        assert_eq!(urls, ["http://desk/press/1", "http://desk/press/2"]);
    }

    /// The wire shape the frontend sends. Pinned because a rule is compiled on
    /// one side and evaluated on the other, and a field misspelled in the
    /// middle is a rule that silently stops narrowing.
    #[test]
    fn a_compiled_rule_arrives_as_a_frontend_writes_it() {
        let rule: CompiledRule = serde_json::from_str(
            r#"{"name":"Go","words":["Go"],"streams":[2],"views":[1],"actions":["highlight","urgentAttention","webhook"],"webhook":{"url":"http://desk:8000/press","method":"post","headers":[],"body":"{{text}}"},"minIntervalMs":500}"#,
        )
        .expect("a compiled rule");

        assert_eq!(rule.streams.as_deref(), Some(&[2u16][..]));
        assert_eq!(rule.views.as_deref(), Some(&[1u16][..]));
        assert_eq!(
            rule.actions,
            [Action::Highlight, Action::UrgentAttention, Action::Webhook]
        );
        assert_eq!(rule.min_interval_ms, 500);
        assert_eq!(
            rule.webhook.as_ref().map(|hook| hook.url.as_str()),
            Some("http://desk:8000/press")
        );
    }
}
