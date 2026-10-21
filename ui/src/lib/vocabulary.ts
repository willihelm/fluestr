// The active vocabulary: the terms whisper is told to expect, for every
// channel of the session.
//
// One list, not one per channel. The jargon of a production — its cue names,
// its departments, its house words — is the same on every talkback, and eight
// copies of one list is eight chances for it to drift. It is also the single
// strongest lever on code-switching quality (measurements.md §4), so a setup
// that applies it to some channels is a setup where the others quietly
// transcribe worse for no reason anyone would look for. See P15.
//
// Named, stored lists with one of them active are still outstanding and belong
// with P10's schema; this is the one active list, editable.

/// German on purpose: this is vocabulary data, not code.
export const DEFAULT_VOCABULARY =
  "Intercom-Kommandos: Standby, Cue, Go, Fade out, Blackout, Talkback, Regie.";

/// Whisper's prompt window, in tokens. Everything past it is dropped by the
/// decoder — silently, and from the *front*, so an over-long list loses the
/// terms typed first rather than the ones typed last.
export const TOKEN_LIMIT = 224;

/// A rough token count for the prompt box's usage meter.
///
/// Four characters to the token is the usual heuristic and it is only that:
/// the real count comes from whisper's own tokeniser, which the UI does not
/// have. German compounds and technical terms split worse than average, so
/// this reads low exactly where it matters — hence the warning at 80 % rather
/// than at the limit itself, and hence `estimated` in the name of everything
/// that carries it.
export const estimatedTokens = (text: string): number =>
  text.trim() === "" ? 0 : Math.ceil(text.trim().length / 4);

/// What to tell the operator about the length of the prompt, or `null` if
/// there is nothing to say.
///
/// A long prompt is not merely truncated: whisper treats it as context and
/// will happily continue it, so a bloated list of terms makes hallucination on
/// quiet channels *more* likely. That is the warning worth giving, and it is
/// worth giving before the limit is reached.
export const lengthWarning = (text: string): string | null => {
  const tokens = estimatedTokens(text);
  if (tokens > TOKEN_LIMIT) {
    return `About ${tokens} tokens, over whisper's limit of ${TOKEN_LIMIT} — the terms at the start will be dropped.`;
  }
  if (tokens > TOKEN_LIMIT * 0.8) {
    return `About ${tokens} of ${TOKEN_LIMIT} tokens. Long prompts also make hallucination on quiet channels more likely.`;
  }
  return null;
};

/// What `start_session` is sent. Empty means no prompt at all rather than an
/// empty one, so a cleared box really does turn the vocabulary off — which is
/// how its effect gets measured (P14).
export const toConfig = (vocabulary: string): string | null =>
  vocabulary.trim() === "" ? null : vocabulary.trim();
