//! The outbound action: one HTTP request per firing rule (P19).
//!
//! This is the action the app is actually wanted for in a theatre, and it is
//! deliberately *one free-text request* rather than a list of integrations.
//! The thing a house points it at is Bitfocus Companion, which is itself a
//! fan-out point — a Companion button reaches every device Companion has a
//! module for, MIDI among them — so the cue still arrives at the desk by a
//! route this app does not have to own. Speaking MIDI directly would have
//! reached what speaks MIDI; speaking HTTP to Companion reaches that and
//! everything else, for less code.
//!
//! But Companion is one destination and not the shape of the feature. Binding
//! the action to it would mean the next thing somebody wants to call — a media
//! server, node-red, a light desk's own gateway, something bought after this
//! was written — needs a release. So there is no preset and no endpoint
//! catalogue: a [`Webhook`] is a URL, a method, a header list and a body, and
//! what is sent is what was typed.
//!
//! **It must not block the transcript.** Rules are evaluated in
//! [`crate::TauriEvents::transcript`], so a synchronous request to an
//! unreachable host would stall transcription for the length of its timeout —
//! and an unreachable host is the *normal* case in a venue with no route out.
//! So [`Sender::send`] never waits: it puts the request on a bounded queue and
//! returns. The queue is drained by one task with a short timeout and **at
//! most one retry**, and then the request is dropped and counted. A cue that
//! arrives forty seconds late is worse than one that never arrives; this is a
//! show, not a message bus. The count is broadcast so that a dead endpoint is
//! visible beside the stream health (P8) rather than silent.
//!
//! **No new dependency.** `reqwest` has been here since P11's model downloads,
//! with rustls and `system-proxy` already on — so an HTTPS destination,
//! including one behind a house proxy, works with what is in the tree. The
//! timestamp below is formatted by hand for the same reason: a date crate for
//! one line of output is a crate to audit and notarise.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Runtime};
use tokio::sync::mpsc;

/// How long a single attempt may take.
///
/// Short on purpose. The destination is a box on the same network as the
/// machine running this — a Companion instance, a media server — and one that
/// has not answered in three seconds is not going to answer in time to be a
/// cue.
const TIMEOUT: Duration = Duration::from_secs(3);

/// How long to wait before the one retry.
///
/// Long enough to ride out a Wi-Fi hiccup, short enough that the retry is
/// still the same moment in the show.
const RETRY_AFTER: Duration = Duration::from_millis(250);

/// How many requests may be waiting to go out.
///
/// Small: a queue that grows is a queue that delivers cues late, which is the
/// one thing this must not do. P19's per-rule rate limit is what keeps a
/// channel trigger on a busy talkback from reaching this at all; the bound is
/// what happens when it does anyway.
const QUEUE: usize = 32;

/// How every value substituted into a header is cleaned, and why a bare
/// `strip` is not enough — see [`header_value`].
const CONTROL: &[char] = &['\r', '\n', '\0'];

/// What a rule calls when it fires.
///
/// Free text throughout, and templated throughout: the URL, every header value
/// and the body all go through [`render`]. That is what lets one mechanism
/// serve both "press a Companion button" and "put what was just said on the
/// button's face" without knowing which is which.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "webhook.ts")]
pub struct Webhook {
    pub url: String,
    pub method: Method,
    #[serde(default)]
    pub headers: Vec<Header>,
    /// **Defaults to the minimum.** `{{text}}` is the exposure — a rule name
    /// and a channel are usually all a trigger needs to say — so a new action
    /// starts with the transcript *out* of the body, and putting it in is a
    /// thing the operator does on purpose. The default should not be the one
    /// that quietly ships what was said in a production off the premises.
    #[serde(default)]
    pub body: String,
}

/// One header, as the operator wrote it.
///
/// A list of pairs rather than a map, because the editor is a list of rows and
/// a half-typed name is a row with an empty name — which a map cannot hold
/// without losing the row the operator is in the middle of writing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "webhook.ts")]
pub struct Header {
    pub name: String,
    pub value: String,
}

/// The methods offered.
///
/// A closed list and not free text: these are the ones a cue endpoint speaks,
/// and a typo in a free-text method box is a rule that fails on the night with
/// a 405 nobody is watching for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "webhook.ts")]
pub enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl Method {
    fn as_reqwest(self) -> reqwest::Method {
        match self {
            Self::Get => reqwest::Method::GET,
            Self::Post => reqwest::Method::POST,
            Self::Put => reqwest::Method::PUT,
            Self::Patch => reqwest::Method::PATCH,
            Self::Delete => reqwest::Method::DELETE,
        }
    }

    /// Whether a body is sent at all. A GET with a body is a request half the
    /// world drops silently, which on a cue is the worst of both.
    fn carries_a_body(self) -> bool {
        !matches!(self, Self::Get)
    }
}

/// The values a template may name.
///
/// The same five the highlight already works over, so that what an operator
/// can put in a body is what they can already see on screen.
#[derive(Debug, Clone, Default)]
pub struct Values {
    /// What was said.
    pub text: String,
    /// The channel's label, as the operator named it — not its stream id. A
    /// number that means something only inside this process is not a thing to
    /// put on a button face.
    pub channel: String,
    /// The rule's name, so that a request arriving at a shared endpoint can
    /// say which rule sent it.
    pub rule: String,
    /// The words that actually matched, joined by a space. Empty for a rule
    /// with no word condition, which has nothing to report here.
    pub matched: String,
    /// When it fired, ISO 8601 in UTC — see [`timestamp`].
    pub timestamp: String,
}

impl Values {
    fn get(&self, name: &str) -> Option<&str> {
        Some(match name {
            "text" => &self.text,
            "channel" => &self.channel,
            "rule" => &self.rule,
            "matched" => &self.matched,
            "timestamp" => &self.timestamp,
            _ => return None,
        })
    }
}

/// One request, rendered and ready to go out.
///
/// Rendered where the rule fired rather than on the sending task, because the
/// values are the segment's and the segment is gone by the time the queue is
/// drained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// Only logged, and never sent: which rule this was, so that a request
    /// arriving somewhere unexpected can be found on the Settings page.
    pub rule: String,
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Webhook {
    /// This webhook with the segment's values in it.
    ///
    /// Each of the three places is escaped for the place it is:
    ///
    /// - the **URL** gets percent-encoding, because a transcript has spaces in
    ///   it and a URL does not;
    /// - a **header value** gets its control characters removed, because a
    ///   newline in a header value is header injection and a transcript is
    ///   text this app did not write;
    /// - the **body** is substituted raw, *unless* a `Content-Type` header
    ///   says JSON — in which case the values are JSON-escaped, because the
    ///   body an operator writes for node-red is `{"said": "{{text}}"}` and a
    ///   quotation mark in what was said would otherwise make it unparseable
    ///   at the far end.
    ///
    /// The template itself is never escaped: only what is put into it.
    pub fn render(&self, values: &Values) -> Request {
        let json_body = self
            .headers
            .iter()
            .any(|h| h.name.eq_ignore_ascii_case("content-type") && h.value.contains("json"));

        Request {
            rule: values.rule.clone(),
            method: self.method,
            url: render(&self.url, values, percent_encode),
            headers: self
                .headers
                .iter()
                .filter(|h| !h.name.trim().is_empty())
                .map(|h| {
                    (
                        h.name.trim().to_string(),
                        render(&h.value, values, header_value),
                    )
                })
                .collect(),
            body: if self.method.carries_a_body() {
                render(
                    &self.body,
                    values,
                    if json_body { json_escape } else { raw },
                )
            } else {
                String::new()
            },
        }
    }
}

/// Substitutes `{{name}}` throughout, escaping each value with `escape`.
///
/// A placeholder nobody defined is left exactly as it was typed rather than
/// replaced with nothing: a body arriving at the far end with `{{txt}}` still
/// in it says what the mistake was, and one arriving with an empty string
/// where the transcript should be says only that the transcript was empty.
fn render(template: &str, values: &Values, escape: fn(&str) -> String) -> String {
    // The common case by far: no placeholder at all, in a URL that is a URL.
    if !template.contains("{{") {
        return template.to_string();
    }

    let mut out = String::with_capacity(template.len());
    let mut rest = template;

    while let Some(open) = rest.find("{{") {
        out.push_str(&rest[..open]);
        let after = &rest[open + 2..];
        let Some(close) = after.find("}}") else {
            // An unclosed `{{` is the operator halfway through typing one.
            out.push_str(&rest[open..]);
            return out;
        };
        let name = after[..close].trim();
        match values.get(name) {
            Some(value) => out.push_str(&escape(value)),
            None => out.push_str(&rest[open..open + 2 + close + 2]),
        }
        rest = &after[close + 2..];
    }

    out.push_str(rest);
    out
}

fn raw(value: &str) -> String {
    value.to_string()
}

/// Percent-encoding for a value going into a URL.
///
/// Conservative: everything outside the unreserved set of RFC 3986 is encoded,
/// so a value can be dropped into a path segment or a query value without
/// knowing which it is. That also means a value cannot smuggle a `?`, a `#` or
/// a `/` and rewrite the request it was put into.
fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// A value going into a header, with what a header may not carry taken out.
///
/// Header injection is the reason this is not `raw`: the values here are
/// transcripts, which is text this app did not write and cannot vouch for, and
/// a newline in a header value ends the header.
fn header_value(value: &str) -> String {
    value.replace(CONTROL, " ")
}

/// A value going into a JSON body, escaped as a JSON string's contents.
///
/// Through `serde_json`, which is already here, rather than by hand: the
/// escaping rules include a control-character range that is easy to get nearly
/// right. The surrounding quotes are trimmed, because the template supplies
/// them — the operator writes `"{{text}}"`, which is what makes the body
/// readable as the JSON it is.
fn json_escape(value: &str) -> String {
    let quoted = serde_json::to_string(value).unwrap_or_else(|_| String::from("\"\""));
    quoted
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(&quoted)
        .to_string()
}

/// Now, as ISO 8601 in UTC: `2026-09-19T20:15:04Z`.
///
/// UTC and not local time, because a time zone needs a database and this crate
/// has no date dependency — see the module documentation. A cue log wants an
/// unambiguous instant more than it wants the wall clock in the room, and the
/// far end can convert.
pub fn timestamp(now: SystemTime) -> String {
    let secs = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let (days, rest) = (secs / 86_400, secs % 86_400);
    let (year, month, day) = civil_from_days(days as i64);
    let (h, m, s) = (rest / 3_600, (rest % 3_600) / 60, rest % 60);
    format!("{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}Z")
}

/// Howard Hinnant's `civil_from_days`, for days since 1970-01-01.
///
/// Copied rather than derived because it is the one piece of calendar
/// arithmetic that is genuinely easy to get subtly wrong, and it is shorter
/// than the explanation of why February needs a special case.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

// --- Getting it out of the building ---------------------------------------

/// How the outbound queue is doing (P8, P19).
///
/// Broadcast whenever it changes, and shown beside the stream health: a dead
/// endpoint that is silently swallowing every cue is the failure this feature
/// can have that nobody would otherwise notice until the cue did not happen.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "webhook.ts", rename = "WebhookStatus")]
pub struct Status {
    /// Requests the far end accepted.
    pub sent: u64,
    /// Requests that were given up on: the queue was full, or both attempts
    /// failed, or the far end answered with an error status. One number,
    /// because from where the operator stands they are one thing — a cue that
    /// did not arrive.
    pub dropped: u64,
    /// Why the last drop happened, in one line, **without the URL in it**.
    ///
    /// Without, because this is broadcast to every view including the browsers
    /// the LAN remote is serving (P16), and the destination is the one part of
    /// a rule the remote is deliberately not shown — see [`Webhook`] and P19.
    pub last_error: Option<String>,
}

/// The queue, its counters, and the handle that drains it.
///
/// Built with [`Default`] as part of [`crate::AppState`], which happens before
/// there is an async runtime to spawn on; [`Sender::start`] is what hands the
/// receiving half to a task. A sender that was never started still accepts
/// requests and drops them when the queue fills, which is what an app with no
/// runtime *should* do with a cue: not block, and say so.
#[derive(Debug)]
pub struct Sender {
    tx: mpsc::Sender<Request>,
    /// Taken by [`Sender::start`]. `None` afterwards, and `None` again if
    /// something tried to start it twice.
    rx: Mutex<Option<mpsc::Receiver<Request>>>,
    sent: AtomicU64,
    dropped: AtomicU64,
    last_error: Mutex<Option<String>>,
}

impl Default for Sender {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel(QUEUE);
        Self {
            tx,
            rx: Mutex::new(Some(rx)),
            sent: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            last_error: Mutex::new(None),
        }
    }
}

impl Sender {
    /// Queues a request, or counts a drop. **Never waits.**
    ///
    /// `try_send` and not `send`: this is called from the thread that is
    /// forwarding a transcript to every view, and a queue that applied
    /// backpressure there would apply it to the chat.
    pub fn send(&self, request: Request) -> bool {
        match self.tx.try_send(request) {
            Ok(()) => true,
            Err(err) => {
                let (request, why) = match err {
                    mpsc::error::TrySendError::Full(request) => {
                        (request, "the outbound queue is full")
                    }
                    mpsc::error::TrySendError::Closed(request) => {
                        (request, "nothing is sending webhooks")
                    }
                };
                tracing::warn!(rule = %request.rule, why, "a webhook was dropped");
                self.drop_one(why.to_string());
                false
            }
        }
    }

    pub fn status(&self) -> Status {
        Status {
            sent: self.sent.load(Ordering::Relaxed),
            dropped: self.dropped.load(Ordering::Relaxed),
            last_error: self.last_error.lock().ok().and_then(|e| e.clone()),
        }
    }

    fn drop_one(&self, why: String) {
        self.dropped.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut last) = self.last_error.lock() {
            *last = Some(why);
        }
    }

    /// Starts the one task that drains the queue.
    ///
    /// Called once, from the app's setup. Each outcome is broadcast, so a
    /// first drop reaches the stream health strip immediately rather than at
    /// the next sample of something else.
    pub fn start<R: Runtime>(state: std::sync::Arc<Self>, app: AppHandle<R>) {
        let Some(mut rx) = state.rx.lock().ok().and_then(|mut slot| slot.take()) else {
            tracing::error!("the webhook queue has already been started");
            return;
        };

        // One client for every request, so that a keep-alive connection to a
        // Companion on the LAN is reused: the second cue of an evening should
        // not pay for a TCP handshake. `timeout` is per attempt, which is what
        // the retry below counts on.
        let client = match reqwest::Client::builder()
            .user_agent(concat!("fluestr/", env!("CARGO_PKG_VERSION")))
            .timeout(TIMEOUT)
            .build()
        {
            Ok(client) => client,
            Err(err) => {
                tracing::error!(error = %err, "no HTTP client — webhooks cannot be sent");
                return;
            }
        };

        tauri::async_runtime::spawn(async move {
            while let Some(request) = rx.recv().await {
                let outcome = deliver(&client, &request).await;
                match outcome {
                    Ok(()) => {
                        state.sent.fetch_add(1, Ordering::Relaxed);
                        tracing::debug!(rule = %request.rule, "webhook sent");
                    }
                    Err(why) => {
                        tracing::warn!(rule = %request.rule, why, "a webhook was dropped");
                        state.drop_one(why);
                    }
                }
                crate::broadcast(&app, crate::EVENT_WEBHOOK_STATUS, state.status());
            }
        });
    }
}

/// One request, with at most one retry, and then a drop.
///
/// Two attempts and not more, because the value of a cue falls off a cliff: a
/// light change that lands forty seconds late is worse than one that never
/// lands, and an operator watching a counter can act on a drop.
async fn deliver(client: &reqwest::Client, request: &Request) -> Result<(), String> {
    let mut last = String::new();
    for attempt in 0..2 {
        if attempt > 0 {
            tokio::time::sleep(RETRY_AFTER).await;
        }
        match attempt_once(client, request).await {
            Ok(()) => return Ok(()),
            Err(why) => last = why,
        }
    }
    Err(last)
}

async fn attempt_once(client: &reqwest::Client, request: &Request) -> Result<(), String> {
    let mut build = client.request(request.method.as_reqwest(), &request.url);
    for (name, value) in &request.headers {
        build = build.header(name, value);
    }
    if request.method.carries_a_body() && !request.body.is_empty() {
        build = build.body(request.body.clone());
    }

    // `without_url` throughout: the message is broadcast to every view, and
    // the destination is the one part of a rule a remote browser is not shown.
    let response = build
        .send()
        .await
        .map_err(|err| err.without_url().to_string())?;

    let status = response.status();
    if status.is_success() {
        Ok(())
    } else {
        Err(format!("the endpoint answered {status}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values() -> Values {
        Values {
            text: "Feuer im Saal".into(),
            channel: "Bühne".into(),
            rule: "Feuer".into(),
            matched: "Feuer".into(),
            timestamp: "2026-09-19T20:15:04Z".into(),
        }
    }

    fn webhook(url: &str, body: &str) -> Webhook {
        Webhook {
            url: url.into(),
            method: Method::Post,
            headers: vec![],
            body: body.into(),
        }
    }

    #[test]
    fn a_template_without_placeholders_is_itself() {
        let request = webhook("http://desk:8000/api/location/1/0/2/press", "").render(&values());
        assert_eq!(request.url, "http://desk:8000/api/location/1/0/2/press");
        assert_eq!(request.body, "");
    }

    /// The second Companion example: what was said, on a button's face. The
    /// thing a MIDI note could never have carried.
    #[test]
    fn the_body_carries_the_transcript() {
        let request = webhook(
            "http://desk:8000/api/custom-variable/last_call/value",
            "{{text}}",
        )
        .render(&values());
        assert_eq!(request.body, "Feuer im Saal");
    }

    #[test]
    fn every_value_is_available() {
        let request = webhook(
            "http://x/",
            "{{rule}}|{{channel}}|{{matched}}|{{timestamp}}",
        )
        .render(&values());
        assert_eq!(request.body, "Feuer|Bühne|Feuer|2026-09-19T20:15:04Z");
    }

    /// A placeholder nobody defined says what the mistake was. Replacing it
    /// with nothing would look like an empty transcript.
    #[test]
    fn an_unknown_placeholder_is_left_alone() {
        let request = webhook("http://x/", "said: {{txt}}").render(&values());
        assert_eq!(request.body, "said: {{txt}}");
    }

    #[test]
    fn a_half_typed_placeholder_does_not_eat_the_rest() {
        let request = webhook("http://x/", "said: {{text").render(&values());
        assert_eq!(request.body, "said: {{text");
    }

    /// A transcript has spaces in it and a URL does not.
    #[test]
    fn a_value_in_the_url_is_percent_encoded() {
        let request = webhook("http://desk:8000/set?v={{text}}", "").render(&values());
        assert_eq!(request.url, "http://desk:8000/set?v=Feuer%20im%20Saal");
    }

    /// And it cannot rewrite the request it was put into.
    #[test]
    fn a_value_cannot_smuggle_a_query_into_the_url() {
        let mut spoken = values();
        spoken.text = "x/../../admin?token=1".into();
        let request = webhook("http://desk:8000/say/{{text}}", "").render(&spoken);
        assert_eq!(
            request.url,
            "http://desk:8000/say/x%2F..%2F..%2Fadmin%3Ftoken%3D1"
        );
    }

    /// The body an operator writes for node-red, with a quotation mark in
    /// what was said: unparseable at the far end without this.
    #[test]
    fn a_json_body_escapes_what_was_said() {
        let mut hook = webhook("http://node-red/hook", r#"{"said":"{{text}}"}"#);
        hook.headers = vec![Header {
            name: "Content-Type".into(),
            value: "application/json".into(),
        }];
        let mut spoken = values();
        spoken.text = "er sagte \"Go\"\nund ging".into();

        let request = hook.render(&spoken);
        assert_eq!(request.body, r#"{"said":"er sagte \"Go\"\nund ging"}"#);
        // And it is JSON again, which is the whole point.
        let parsed: serde_json::Value = serde_json::from_str(&request.body).expect("valid JSON");
        assert_eq!(parsed["said"], "er sagte \"Go\"\nund ging");
    }

    /// Without a JSON content type the body is what was typed: a form post, a
    /// bare string on a Companion variable, an XML envelope.
    #[test]
    fn a_body_that_is_not_json_is_substituted_raw() {
        let mut spoken = values();
        spoken.text = "er sagte \"Go\"".into();
        let request = webhook("http://desk/v", "{{text}}").render(&spoken);
        assert_eq!(request.body, "er sagte \"Go\"");
    }

    /// A transcript is text this app did not write, and a newline in a header
    /// value ends the header.
    #[test]
    fn a_header_value_cannot_carry_a_newline() {
        let mut hook = webhook("http://x/", "");
        hook.headers = vec![Header {
            name: " X-Said ".into(),
            value: "{{text}}".into(),
        }];
        let mut spoken = values();
        spoken.text = "a\r\nX-Admin: yes".into();

        let request = hook.render(&spoken);
        assert_eq!(
            request.headers,
            [("X-Said".to_string(), "a  X-Admin: yes".to_string())]
        );
    }

    /// A row the operator is halfway through writing is not a header.
    #[test]
    fn a_nameless_header_is_left_out() {
        let mut hook = webhook("http://x/", "");
        hook.headers = vec![Header {
            name: "  ".into(),
            value: "something".into(),
        }];
        assert!(hook.render(&values()).headers.is_empty());
    }

    /// A GET with a body is a request half the world drops silently.
    #[test]
    fn a_get_sends_no_body() {
        let mut hook = webhook("http://desk/press", "{{text}}");
        hook.method = Method::Get;
        assert_eq!(hook.render(&values()).body, "");
    }

    #[test]
    fn the_timestamp_is_iso_8601_in_utc() {
        // 2026-09-19T20:15:04Z
        let at = UNIX_EPOCH + Duration::from_secs(1_789_848_904);
        assert_eq!(timestamp(at), "2026-09-19T20:15:04Z");
        // The epoch itself, and a leap day, because that is where a hand-rolled
        // calendar goes wrong.
        assert_eq!(timestamp(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        assert_eq!(
            timestamp(UNIX_EPOCH + Duration::from_secs(1_709_164_800)),
            "2024-02-29T00:00:00Z"
        );
    }

    /// The queue is bounded, and full means dropped rather than waited on:
    /// this is called from the thread forwarding transcripts to every view.
    #[test]
    fn a_full_queue_drops_rather_than_waits() {
        let sender = Sender::default();
        let request = Request {
            rule: "Go".into(),
            method: Method::Post,
            url: "http://desk/press".into(),
            headers: vec![],
            body: String::new(),
        };

        for _ in 0..QUEUE {
            assert!(sender.send(request.clone()));
        }
        assert!(!sender.send(request));

        let status = sender.status();
        assert_eq!(status.sent, 0);
        assert_eq!(status.dropped, 1);
        assert_eq!(
            status.last_error.as_deref(),
            Some("the outbound queue is full")
        );
    }

    /// The wire shape, pinned: a rule is written on one side and evaluated on
    /// the other, and a field misspelled in the middle is a cue that never
    /// fires.
    #[test]
    fn a_webhook_arrives_as_a_frontend_writes_it() {
        let hook: Webhook = serde_json::from_str(
            r#"{"url":"http://desk:8000/api/location/1/0/2/press","method":"post","headers":[{"name":"X-Cue","value":"{{rule}}"}],"body":""}"#,
        )
        .expect("a webhook");
        assert_eq!(hook.method, Method::Post);
        assert_eq!(hook.headers.len(), 1);
    }
}
