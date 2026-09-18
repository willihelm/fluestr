// Asking to be looked at, in a browser tab (P16, P17).
//
// The app asks the platform: macOS bounces the Dock icon, and keeps bouncing
// while an alert term is waiting. A page served over the LAN has no platform
// to ask — it is one tab among the operator's others, quite possibly behind
// them — so it asks with the two things a tab does own: its own surface, and
// its title in the tab strip.
//
// Same rules as the window's request, deliberately. An **alert term** is
// urgent and keeps asking until the page is looked at; a group's
// flash-on-message is a single pulse. Never a sound: the far end of a remote
// is still somebody sitting in a room full of speech, and often one where a
// beep from a laptop is the last thing wanted.
//
// A visible page is left alone, which is the browser's version of "the window
// is focused": an operator already looking at the chat is not asked to look at
// it.
//
// `floodWindow` is the exception on both counts. It is the loud request a
// group's flash setting makes — the whole surface in dark orange — and it is
// drawn by the page in the app as well, because in the app the page *is* the
// window and there is nothing else that could paint it.

/// How long one pulse of the page lasts, and how fast the title alternates.
///
/// A pulse long enough to catch the eye of somebody looking somewhere else,
/// and a title that changes about twice a second — fast enough to be movement
/// in the tab strip, slow enough to be read.
const PULSE_MS = 1_200;
const TITLE_INTERVAL_MS = 600;

/// How long the window stays flooded. The same pulse, because it is the same
/// request made louder — a flood that outlasted the pulse would be a different
/// kind of ask, and the operator would learn two timings for one event.
const FLOOD_MS = PULSE_MS;

/// What the title is prefixed with while the page is asking. A shape rather
/// than a word, because the rest of the title is the group's name and an
/// operator scanning eight tabs reads the mark, not the sentence.
const MARK = "● ";

let titleTimer: ReturnType<typeof setInterval> | null = null;
let pulseTimer: ReturnType<typeof setTimeout> | null = null;

/// Whether the page is still asking to be looked at. Urgent requests keep
/// asking; a single pulse stops on its own.
let insisting = false;

/// The mark is added to and taken off the title as it stands, never restored
/// from a copy taken when the flashing started: `setWindowTitle` can rename
/// the page while it is flashing — a group renamed on the Settings page — and
/// putting the old name back afterwards would undo that rename.
const marked = (title: string) => (title.startsWith(MARK) ? title : MARK + title);
const unmarked = (title: string) => (title.startsWith(MARK) ? title.slice(MARK.length) : title);

/// Stops asking, and puts the title back the way it was.
///
/// Called when the page is looked at, and by the end of a single pulse. Idempotent,
/// because both of those can happen in either order.
export const stopFlashing = () => {
  insisting = false;
  if (titleTimer !== null) {
    clearInterval(titleTimer);
    titleTimer = null;
  }
  if (pulseTimer !== null) {
    clearTimeout(pulseTimer);
    pulseTimer = null;
  }
  document.title = unmarked(document.title);
  document.body.classList.remove("flashing", "urgent");
};

/// Asks for the operator's attention.
///
/// `urgent` is an alert term, and it keeps asking until the tab is looked at.
/// Anything else is one pulse — the quieter request a group's
/// flash-on-message makes.
export const flashPage = (urgent: boolean) => {
  // Already being looked at. Not a request to decline politely: it is the one
  // case where flashing would be noise with nothing behind it.
  if (document.visibilityState === "visible" && document.hasFocus()) return;

  document.body.classList.add("flashing");
  document.body.classList.toggle("urgent", urgent);

  if (titleTimer === null) {
    document.title = marked(document.title);
    titleTimer = setInterval(() => {
      document.title = document.title.startsWith(MARK)
        ? unmarked(document.title)
        : marked(document.title);
    }, TITLE_INTERVAL_MS);
  }

  if (urgent) {
    insisting = true;
    // An urgent request outlives a pulse that was already running: a group
    // message followed by an alert term must not stop asking because the
    // quieter of the two got there first.
    if (pulseTimer !== null) {
      clearTimeout(pulseTimer);
      pulseTimer = null;
    }
    return;
  }

  if (insisting || pulseTimer !== null) return;
  pulseTimer = setTimeout(() => {
    pulseTimer = null;
    if (!insisting) stopFlashing();
  }, PULSE_MS);
};

// --- The loud one --------------------------------------------------------

let floodTimer: ReturnType<typeof setTimeout> | null = null;

/// Floods the whole window in dark orange for one pulse.
///
/// What a group with its flash setting on asks for, and the one request that
/// is made **whether or not the window is focused**. Everything else here
/// declines to ask an operator who is already looking; this setting exists
/// precisely for the view that must be noticed while they are looking
/// somewhere else on the same screen — the chat is one panel of a window that
/// also holds meters, a sidebar and a settings page.
///
/// A pulse per message rather than a state that persists until the window is
/// looked at: the quantity being signalled is "another one arrived", and a
/// window that stayed orange would stop counting after the first.
export const floodWindow = () => {
  // Restarted rather than extended. A second message during the first flood
  // has to be visible as a second flood, and re-adding a class the element
  // already has does not replay its animation — the reflow between the two is
  // what makes the browser treat it as a new one.
  document.body.classList.remove("flooding");
  void document.body.offsetWidth;
  document.body.classList.add("flooding");

  if (floodTimer !== null) clearTimeout(floodTimer);
  floodTimer = setTimeout(() => {
    floodTimer = null;
    document.body.classList.remove("flooding");
  }, FLOOD_MS);
};

/// Stops the flashing as soon as the page is actually looked at.
///
/// Both events, because they are different things: switching to the tab is
/// `visibilitychange`, clicking into a window that was already showing the tab
/// is `focus`. An operator who has seen it has seen it, either way.
export const watchForAttention = () => {
  const seen = () => {
    if (document.visibilityState === "visible") stopFlashing();
  };
  document.addEventListener("visibilitychange", seen);
  window.addEventListener("focus", seen);
  return () => {
    document.removeEventListener("visibilitychange", seen);
    window.removeEventListener("focus", seen);
  };
};
