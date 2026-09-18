// Whether somebody is in the middle of typing something.
//
// The configuration is saved as a consequence of the state changing, not of a
// button — there is no Save on the Settings page, because a setup an operator
// believes is stored and is not is worse than no persistence at all. What that
// costs is that a save lands *while a field is being typed in*, and a save is
// what every view rebuilds its Settings page from (P16). The field under the
// cursor is rebuilt with it, and a label then has to be typed one character at
// a time, chasing the focus back into the box after each one.
//
// So the save waits for the field to be left. Not for a button — leaving the
// field is the operator saying they are done with it, and it is something they
// were going to do anyway.
//
// Only *typed* fields. A colour, a language, a device: changing one of those
// is a single discrete act with nothing half-finished about it, and making it
// wait for a focus change would be making it wait for nothing.

/// The parts of an element this question turns on. An interface rather than
/// `Element`, so the rule can be tested without a DOM to build one in.
export interface Focusable {
  tagName: string;
  type?: string;
  isContentEditable?: boolean;
}

/// Input types that are typed into a character at a time.
///
/// Listed rather than excluded, because the set that is *not* typed into — a
/// colour, a checkbox, a radio, a range — is the set that grows every time the
/// platform adds a control, and a new one would silently start deferring its
/// saves until something else happened to take the focus.
const TYPED = ["text", "number", "search", "url", "email", "tel", "password"];

/// Whether this element is one somebody types into.
export const isEditable = (element: Focusable | null | undefined): boolean => {
  if (!element) return false;
  if (element.isContentEditable) return true;

  const tag = element.tagName.toUpperCase();
  if (tag === "TEXTAREA") return true;
  if (tag !== "INPUT") return false;

  // A bare `<input>` is a text field; the attribute is what narrows it.
  return TYPED.includes((element.type ?? "text").toLowerCase());
};

/// Whether the page's focus is currently in a field being typed into.
export const isEditing = (): boolean =>
  typeof document !== "undefined" &&
  isEditable(document.activeElement as Focusable | null);

/// Runs `fn` once the current field is left, or straight away when nothing is
/// being typed into.
///
/// Any `focusout` counts, including moving to the next field and including the
/// window losing focus altogether: in all of them the operator is done with
/// the box they were in, which is the whole of what this waits for.
///
/// Returns a cancel, because the usual reason to stop waiting is another
/// keystroke — the caller re-arms with the newer configuration rather than
/// saving the one it was holding.
export const whenEditingEnds = (fn: () => void): (() => void) => {
  if (!isEditing()) {
    fn();
    return () => {};
  }

  const done = () => {
    document.removeEventListener("focusout", done);
    fn();
  };
  document.addEventListener("focusout", done);
  return () => document.removeEventListener("focusout", done);
};

/// Runs `fn` when the page is going away.
///
/// The net under the rule above: a window closed with the cursor still in a
/// label would otherwise take that label with it. Best effort, deliberately —
/// `pagehide` is the last moment a page is reliably told anything, and what it
/// can start it cannot wait for. It is the difference between usually keeping
/// that last edit and never keeping it.
export const beforeLeaving = (fn: () => void): (() => void) => {
  const leaving = () => fn();
  window.addEventListener("pagehide", leaving);
  return () => window.removeEventListener("pagehide", leaving);
};
