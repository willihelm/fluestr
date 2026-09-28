// How a keyboard shortcut is written for whoever is reading it (P18).
//
// The handler in App.svelte answers ⌘ and Ctrl alike, so what differs between
// platforms is only the label. Read from the page's own browser rather than
// the backend's platform: the LAN remote (P16) may be a Windows laptop looking
// at a Mac, and the hint has to name the key on the keyboard in front of it.

/// Whether the reader's keyboard has a ⌘ key.
export function isMac(userAgent: string = globalThis.navigator?.userAgent ?? ""): boolean {
  return /Mac|iPhone|iPad/.test(userAgent);
}

/// `key` with the platform's modifier: "⌘T" on a Mac, "Ctrl+T" elsewhere.
export function shortcut(key: string, mac: boolean = isMac()): string {
  return mac ? `⌘${key}` : `Ctrl+${key}`;
}
