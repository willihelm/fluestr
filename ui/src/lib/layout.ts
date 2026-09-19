// Where the layout stops being a desk and starts being a phone.
//
// The app runs in two rather different rooms. On the desk it is a window with
// a sidebar, a strip of meters and a chat, all on screen at once. Over the LAN
// remote (P16) it is as likely to be a phone held in one hand in the
// auditorium, where a permanent sidebar is a third of the screen spent on a
// list that is read once, and eight horizontal meters are eight lines that
// leave no room for the thing they are above.
//
// One breakpoint for the whole interface, kept here rather than typed into
// each component's `@media`: the chat sidebar, the Settings rail and the
// meters all have to change at the same width, or the window passes through a
// size where the burger is showing and the drawer it opens is still laid out
// as a column.
//
// The CSS side of that rule is written out literally in each component —
// `@media (max-width: 45rem)` — because a media query cannot read a custom
// property. `NARROW_QUERY` is what the *script* side matches on, and the two
// are the same number: 45 rem at the app's 15 px root is 675 px.

/// The widest window that is still "narrow". A phone in landscape is under it;
/// the app beside a cue sheet on half a laptop screen is around it, which is
/// deliberate — that window has the same problem a phone does.
export const NARROW_PX = 675;

/// What `matchMedia` is asked. In pixels rather than rem because a media query
/// in `matchMedia` resolves rem against the root font size, which the app sets
/// and which a browser's own text-size setting then scales.
export const NARROW_QUERY = `(max-width: ${NARROW_PX}px)`;

/// Whether a window of this width gets the narrow layout.
///
/// The decision, separate from the browser that asks it — the same split as
/// `sidebar.ts`: this is the part with a rule in it, and `watchNarrow` below
/// is three lines of `matchMedia` with nothing to get wrong.
export const isNarrow = (available: number): boolean =>
  Number.isFinite(available) && available <= NARROW_PX;

/// Calls back with the current answer and again whenever it changes, and
/// returns the unsubscribe.
///
/// Read through `matchMedia` rather than a `resize` listener on
/// `window.innerWidth`: this fires once when the answer flips rather than on
/// every pixel of a window drag, and a phone rotating is one event either way.
export const watchNarrow = (onChange: (narrow: boolean) => void): (() => void) => {
  // A test environment, or an ancient webview. The desk layout is the one that
  // works at any width, so it is what "we cannot tell" resolves to.
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") {
    onChange(false);
    return () => {};
  }

  const query = window.matchMedia(NARROW_QUERY);
  onChange(query.matches);

  const listener = (event: MediaQueryListEvent) => onChange(event.matches);
  query.addEventListener("change", listener);
  return () => query.removeEventListener("change", listener);
};
