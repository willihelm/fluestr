// The remote's optional key (P16).
//
// Generated here rather than in the backend, and that is not a shortcut. The
// configuration has exactly one writer — the Settings page — and a key the
// backend invented during a save would be a second one: the page would send a
// configuration, get a different one back, and adopt it. Everything the
// backend does with the key is compare it.
//
// Short on purpose. It has to survive being read off a screen, typed on a
// phone and said out loud across a room, and what makes it safe enough is not
// its length but the rate limit in front of it — see `MAX_FAILURES` in
// `crates/app/src/remote.rs`. Eight characters of a 32-letter alphabet is
// forty bits, which ten guesses a minute will not get through.

/// How long a generated key is.
export const KEY_LENGTH = 8;

/// The alphabet, which is Crockford's base 32: the digits and the letters,
/// without `I`, `L`, `O` and `U`.
///
/// `I`/`1` and `O`/`0` are the two pairs somebody reading a key off a screen
/// gets wrong, and `U` is left out so that no key ever spells anything. Exactly
/// 32 letters, so a byte maps onto it without favouring the first few.
const ALPHABET = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// A fresh key.
///
/// From `crypto.getRandomValues`, which — unlike `crypto.subtle` — a browser
/// offers on a plain http page too, and this page is http on a LAN address.
/// Nothing falls back to `Math.random`: a key that is quietly not random is
/// worse than a button that says it could not make one, because it looks
/// exactly like a key that is.
export const newKey = (): string => {
  const bytes = new Uint8Array(KEY_LENGTH);
  crypto.getRandomValues(bytes);
  let key = "";
  // `% 32` and not `% 31`: the alphabet's length divides 256, so every letter
  // is as likely as every other one.
  for (const byte of bytes) key += ALPHABET[byte % ALPHABET.length];
  return key;
};
