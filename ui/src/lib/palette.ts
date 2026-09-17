// Channel colours.
//
// A channel is identified by its colour long before anyone reads its label —
// on the meter strip, on the badge in front of every line, and later on the
// group sidebar. So the colour is part of the channel's configuration, not a
// rendering detail: one is preselected from this palette when a channel is
// created, and the operator can change it (P12 persists that choice).
//
// Eight hues for eight channels, spread far enough apart to stay distinct, and
// all light enough to read as text on the dark background the app runs on.
export const PALETTE = [
  "#6ea8fe", // blue
  "#5fd38d", // green
  "#f2b544", // amber
  "#e879a6", // pink
  "#9b8cf5", // violet
  "#4fc9d8", // cyan
  "#f08a5d", // orange
  "#b5c94f", // lime
] as const;

// Deterministic, so adding a channel never reshuffles the ones already on
// screen. Wraps rather than running out: past eight channels two share a hue,
// which is a legibility problem and not a failure.
export const paletteColor = (index: number): string =>
  PALETTE[index % PALETTE.length];
