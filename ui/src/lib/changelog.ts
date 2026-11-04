// What an update's release notes say, in the shape the changelog dialog draws.
//
// The notes are Markdown, written by `scripts/release-notes.sh`: `### `
// headings over `- ` lists. Read here rather than handed to a Markdown library
// and put into the page as HTML: the dialog needs headings and items and
// nothing else, and text that never becomes markup cannot inject any. Notes
// written by hand on GitHub come through as well — whatever is not a heading
// or an item is kept as a line of text, with the emphasis and the link syntax
// taken off.

export type Block =
  | { kind: "heading"; text: string }
  | { kind: "item"; text: string }
  | { kind: "text"; text: string };

/// Inline Markdown reduced to its words: `[text](url)` to its text, and the
/// `**`, `__` and backticks around a word dropped.
const plain = (line: string): string =>
  line
    .replace(/\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/(\*\*|__|`)/g, "")
    .trim();

/// A line that is only a link to elsewhere — GitHub's "Full Changelog:
/// <compare URL>" above all. There is nothing in the dialog to follow it with.
const onlyLink = (line: string): boolean =>
  /^(\*\*)?full changelog(\*\*)?:/i.test(line) || /^<?https?:\/\/\S+>?$/.test(line);

export const parseNotes = (notes: string | null): Block[] => {
  if (!notes) return [];
  const blocks: Block[] = [];
  for (const raw of notes.split(/\r?\n/)) {
    const line = raw.trim();
    if (line === "" || onlyLink(line)) continue;
    const heading = /^#{1,6}\s+(.*)$/.exec(line);
    if (heading) {
      blocks.push({ kind: "heading", text: plain(heading[1]) });
      continue;
    }
    const item = /^[-*+]\s+(.*)$/.exec(line);
    if (item) {
      blocks.push({ kind: "item", text: plain(item[1]) });
      continue;
    }
    blocks.push({ kind: "text", text: plain(line) });
  }
  return blocks.filter((block) => block.text !== "");
};

const SEEN_KEY = "fluestr.update.seen";

/// Whether the dialog has already been shown for this version, on this
/// machine. It opens by itself once per release; after "Later" the header's
/// button is the way back to it.
export const alreadySeen = (version: string): boolean => {
  try {
    return window.localStorage.getItem(SEEN_KEY) === version;
  } catch {
    return false;
  }
};

export const markSeen = (version: string) => {
  try {
    window.localStorage.setItem(SEEN_KEY, version);
  } catch {
    // No storage — it opens again at the next start, which is harmless.
  }
};
