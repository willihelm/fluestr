// Where a ticker was left (P24), and what it shows.
//
// Remembered per view and per screen, in `localStorage`, like the sidebar
// width and for its reason: where a strip sits beside the DAW is a property of
// the laptop at FOH, and one stored in the configuration would put the desk's
// ticker wherever some other machine last dragged it. The ticker writes it as
// it is moved and resized; the page that opens it reads it and hands it to the
// backend, which checks that the place still exists before using it — see
// `crates/app/src/ticker.rs`.

import type { GroupView } from "./windows";

/// Where a ticker was, in physical pixels — what the window's own move and
/// resize events report, so nothing is converted on the way into storage and
/// nothing is rounded on the way back. Mirrors `ticker::Geometry` in Rust.
export interface Geometry {
  x: number;
  y: number;
  width: number;
  height: number;
}

/// How many bubbles the ticker lays out. More than fit, on purpose: the window
/// clips the oldest at its far edge, so a tall ticker shows more of them and a
/// one-line ticker shows the newest — without the page having to measure
/// anything.
export const LINES = 6;

export const geometryKey = (view: GroupView): string => `fluestr.ticker.${view}`;

const whole = (value: unknown): value is number =>
  typeof value === "number" && Number.isInteger(value);

/// What a stored entry means, including the ways there is none: never written,
/// written by something else, or not JSON at all.
export const readGeometry = (stored: string | null): Geometry | null => {
  if (stored === null) return null;

  let parsed: unknown;
  try {
    parsed = JSON.parse(stored);
  } catch {
    return null;
  }
  if (typeof parsed !== "object" || parsed === null) return null;

  const { x, y, width, height } = parsed as Record<string, unknown>;
  if (!whole(x) || !whole(y) || !whole(width) || !whole(height)) return null;
  if (width <= 0 || height <= 0) return null;

  return { x, y, width, height };
};

/// This view's ticker's last place, or `null` for the default.
///
/// Storage is reached inside a `try`, for the sidebar's reason: a webview with
/// storage disabled throws on the access, and a ticker that fails to open
/// because it could not remember where it was would be absurd.
export const loadGeometry = (view: GroupView): Geometry | null => {
  try {
    return readGeometry(window.localStorage.getItem(geometryKey(view)));
  } catch {
    return null;
  }
};

/// Remembers where this view's ticker is. Silent when it cannot: the ticker is
/// where it was put, which is what was actually being asked for.
export const saveGeometry = (view: GroupView, geometry: Geometry) => {
  try {
    window.localStorage.setItem(geometryKey(view), JSON.stringify(geometry));
  } catch {
    // No storage — the next ticker opens at the default place.
  }
};

/// An edge the ticker can be resized by. Never the left one: that edge is the
/// grip, and a grip whose outer pixels resize the window instead of moving it
/// is a grip that does the wrong thing half the time it is caught.
export type Edge = "top" | "right" | "bottom" | "top-right" | "bottom-right";

/// The smallest ticker, in logical pixels: one line, and enough of it to hold
/// a label and a few words. Kept in step with `MIN_WIDTH` and `MIN_HEIGHT` in
/// `crates/app/src/ticker.rs`.
export const MIN_WIDTH = 240;
export const MIN_HEIGHT = 32;

/// Where the window goes when `edge` has been dragged by `dx`, `dy` physical
/// pixels from `start` — a drag that began with the window at `start`, so a
/// drag is always measured from where it began and never drifts.
///
/// Held to the minimum at the edge being dragged: dragging the top edge down
/// past the minimum stops the window rather than pushing it down the screen.
export const resized = (
  start: Geometry,
  edge: Edge,
  dx: number,
  dy: number,
  scale: number,
): Geometry => {
  const minWidth = Math.round(MIN_WIDTH * scale);
  const minHeight = Math.round(MIN_HEIGHT * scale);
  const next = { ...start };

  if (edge === "right" || edge === "top-right" || edge === "bottom-right") {
    next.width = Math.max(minWidth, Math.round(start.width + dx));
  }
  if (edge === "bottom" || edge === "bottom-right") {
    next.height = Math.max(minHeight, Math.round(start.height + dy));
  }
  if (edge === "top" || edge === "top-right") {
    next.height = Math.max(minHeight, Math.round(start.height - dy));
    next.y = start.y + start.height - next.height;
  }
  return next;
};
