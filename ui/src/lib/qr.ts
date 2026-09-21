// A QR code for the remote client's address (P16).
//
// The address is typed into the other machine's browser, and when that machine
// is a phone, typing `http://192.168.1.23:8420` on a glass keyboard while
// standing at a rack is the worst part of setting the remote up. A camera
// reads it instead.
//
// Written out rather than taken from a package. The whole job is one short
// URL — byte mode, one error-correction level, and the handful of versions a
// URL ever reaches — and the code is served to a browser on a network with no
// route to the internet, out of a bundle embedded in the binary. A dependency
// that has to be vendored, audited and kept current for two hundred lines of
// finite-field arithmetic that has not changed since 2006 is the worse trade.
//
// What is deliberately *not* here: numeric, alphanumeric and kanji modes, ECC
// levels other than M, and versions past 9. Byte mode encodes anything, M is
// the level the standard treats as ordinary, and version 9 holds 179 bytes —
// seven times the longest address this can ever be asked for. Anything longer
// is not a URL that went unnoticed, it is a bug, and it throws.

/// The one error-correction level, and the two bits that name it in the
/// format information. Level M recovers about 15% of the code, which is what
/// a camera pointed at a lit screen from across a room needs.
const ECC_M_BITS = 0b00;

/// Total codewords, error-correction codewords per block, and the number of
/// blocks, for versions 1 to 9 at level M. Straight from the standard's
/// tables; blocks of two sizes are handled by spreading the data codewords
/// across `blocks` as evenly as the table says (the short blocks first).
const VERSIONS = [
  { total: 26, eccPerBlock: 10, blocks: 1 },
  { total: 44, eccPerBlock: 16, blocks: 1 },
  { total: 70, eccPerBlock: 26, blocks: 1 },
  { total: 100, eccPerBlock: 18, blocks: 2 },
  { total: 134, eccPerBlock: 24, blocks: 2 },
  { total: 172, eccPerBlock: 16, blocks: 4 },
  { total: 196, eccPerBlock: 18, blocks: 4 },
  { total: 242, eccPerBlock: 22, blocks: 4 },
  { total: 292, eccPerBlock: 22, blocks: 5 },
];

/// Where the alignment patterns sit, per version. Every pair of these
/// coordinates is a centre, except the three that would land on a finder
/// pattern.
const ALIGNMENT = [
  [],
  [6, 18],
  [6, 22],
  [6, 26],
  [6, 30],
  [6, 34],
  [6, 22, 38],
  [6, 24, 42],
  [6, 26, 46],
];

// --- The field -----------------------------------------------------------

/// Exponent and logarithm tables for GF(256) with the standard primitive
/// polynomial, which is what turns multiplication into addition of logs. The
/// exponent table is doubled so a sum of two logs never has to be reduced.
const EXP = new Uint8Array(512);
const LOG = new Uint8Array(256);
{
  let x = 1;
  for (let i = 0; i < 255; i += 1) {
    EXP[i] = x;
    LOG[x] = i;
    x <<= 1;
    if (x & 0x100) x ^= 0x11d;
  }
  for (let i = 255; i < 512; i += 1) EXP[i] = EXP[i - 255];
}

const mul = (a: number, b: number): number =>
  a === 0 || b === 0 ? 0 : EXP[LOG[a] + LOG[b]];

/// The generator polynomial of the given degree: the product of (x - α^i) for
/// i below the degree, highest power first.
const generator = (degree: number): Uint8Array => {
  let poly = new Uint8Array([1]);
  for (let i = 0; i < degree; i += 1) {
    const next = new Uint8Array(poly.length + 1);
    for (let j = 0; j < poly.length; j += 1) {
      next[j] ^= poly[j];
      next[j + 1] ^= mul(poly[j], EXP[i]);
    }
    poly = next;
  }
  return poly;
};

/// The error-correction codewords for one block: the remainder of the data
/// divided by the generator polynomial.
const remainder = (data: Uint8Array, count: number): Uint8Array => {
  const gen = generator(count);
  const rem = new Uint8Array(count);
  for (const byte of data) {
    const factor = byte ^ rem[0];
    rem.copyWithin(0, 1);
    rem[count - 1] = 0;
    for (let i = 0; i < count; i += 1) rem[i] ^= mul(gen[i + 1], factor);
  }
  return rem;
};

// --- The data ------------------------------------------------------------

/// The codewords for `text`, at the smallest version that holds it: the byte
/// mode header, the text, the padding the standard prescribes, and then the
/// blocks and their error correction interleaved the way the placement step
/// reads them.
const codewords = (text: string): { version: number; data: Uint8Array } => {
  const bytes = new TextEncoder().encode(text);

  // Four bits of mode and eight of length, which is the length field's width
  // for every version up to 9 — the reason the table stops there.
  const needed = bytes.length + 2;
  const index = VERSIONS.findIndex(
    (v) => v.total - v.eccPerBlock * v.blocks >= needed,
  );
  if (index < 0) {
    throw new Error(`too long for a QR code of version 9: ${bytes.length} bytes`);
  }
  const { total, eccPerBlock, blocks } = VERSIONS[index];
  const dataCount = total - eccPerBlock * blocks;

  const buffer = new Uint8Array(dataCount);
  let at = 0;
  const push = (value: number, width: number) => {
    for (let i = width - 1; i >= 0; i -= 1) {
      if ((value >> i) & 1) buffer[at >> 3] |= 0x80 >> (at & 7);
      at += 1;
    }
  };

  push(0b0100, 4);
  push(bytes.length, 8);
  for (const byte of bytes) push(byte, 8);
  // The terminator, as much of it as fits, and then up to a whole byte.
  push(0, Math.min(4, dataCount * 8 - at));
  if (at & 7) push(0, 8 - (at & 7));
  // The two pad codewords, alternating, until the capacity is full.
  for (let i = 0; at < dataCount * 8; i += 1) push(i % 2 === 0 ? 0xec : 0x11, 8);

  // Blocks come in two sizes at most, and the longer ones come last.
  const longBlocks = dataCount % blocks;
  const shortLength = Math.floor(dataCount / blocks);
  const pieces: { data: Uint8Array; ecc: Uint8Array }[] = [];
  for (let i = 0, from = 0; i < blocks; i += 1) {
    const length = shortLength + (i >= blocks - longBlocks ? 1 : 0);
    const block = buffer.subarray(from, from + length);
    from += length;
    pieces.push({ data: block, ecc: remainder(block, eccPerBlock) });
  }

  // Interleaved: the first codeword of every block, then the second, and so
  // on, and the same for the error correction. A short block simply has
  // nothing to give on the last pass.
  const out = new Uint8Array(total);
  let write = 0;
  for (let i = 0; i <= shortLength; i += 1) {
    for (const piece of pieces) if (i < piece.data.length) out[write++] = piece.data[i];
  }
  for (let i = 0; i < eccPerBlock; i += 1) {
    for (const piece of pieces) out[write++] = piece.ecc[i];
  }

  return { version: index + 1, data: out };
};

// --- The matrix ----------------------------------------------------------

/// A code, as a square of booleans: `true` is a dark module. The quiet zone
/// around it is not included — that is the drawing's business, not the
/// encoding's.
export interface QrCode {
  size: number;
  modules: boolean[][];
}

/// Lays the function patterns into a fresh matrix, and marks every module
/// they occupy — including the format and version areas — as taken, so the
/// data has somewhere to go and the mask knows what to leave alone.
const skeleton = (version: number) => {
  const size = version * 4 + 17;
  const modules: boolean[][] = Array.from({ length: size }, () =>
    new Array<boolean>(size).fill(false),
  );
  const taken: boolean[][] = Array.from({ length: size }, () =>
    new Array<boolean>(size).fill(false),
  );

  const set = (row: number, col: number, dark: boolean) => {
    modules[row][col] = dark;
    taken[row][col] = true;
  };

  // The three finders, each with the separator that keeps it apart from the
  // data. The loop runs one module wide on every side to draw that separator,
  // and skips whatever falls outside the matrix.
  for (const [top, left] of [
    [0, 0],
    [0, size - 7],
    [size - 7, 0],
  ]) {
    for (let r = -1; r <= 7; r += 1) {
      for (let c = -1; c <= 7; c += 1) {
        const row = top + r;
        const col = left + c;
        if (row < 0 || row >= size || col < 0 || col >= size) continue;
        const ring = Math.max(Math.abs(r - 3), Math.abs(c - 3));
        set(row, col, ring !== 2 && ring <= 3);
      }
    }
  }

  // The two timing patterns, which are what a reader measures the module
  // pitch against.
  for (let i = 8; i < size - 8; i += 1) {
    set(6, i, i % 2 === 0);
    set(i, 6, i % 2 === 0);
  }

  // The alignment patterns, at every pair of the version's coordinates
  // except the three corners already held by a finder.
  const centres = ALIGNMENT[version - 1];
  for (const row of centres) {
    for (const col of centres) {
      if ((row === 6 && col === 6) || (row === 6 && col === size - 7) ||
          (row === size - 7 && col === 6)) {
        continue;
      }
      for (let r = -2; r <= 2; r += 1) {
        for (let c = -2; c <= 2; c += 1) {
          set(row + r, col + c, Math.max(Math.abs(r), Math.abs(c)) !== 1);
        }
      }
    }
  }

  // The dark module, which is always set and always here.
  set(size - 8, 8, true);

  // The format information goes in twice, and both copies are reserved now
  // and written once a mask has been chosen.
  for (let i = 0; i <= 8; i += 1) {
    if (!taken[8][i]) set(8, i, false);
    if (!taken[i][8]) set(i, 8, false);
  }
  for (let i = 0; i < 8; i += 1) {
    if (!taken[8][size - 1 - i]) set(8, size - 1 - i, false);
    if (!taken[size - 1 - i][8]) set(size - 1 - i, 8, false);
  }

  // Version information, from version 7 on: two copies, beside the top-right
  // and bottom-left finders.
  if (version >= 7) {
    let bits = version;
    for (let i = 0; i < 12; i += 1) {
      bits = (bits << 1) ^ ((bits >> 11) * 0x1f25);
    }
    const info = (version << 12) | bits;
    for (let i = 0; i < 18; i += 1) {
      const dark = ((info >> i) & 1) === 1;
      set(size - 11 + (i % 3), Math.floor(i / 3), dark);
      set(Math.floor(i / 3), size - 11 + (i % 3), dark);
    }
  }

  return { size, modules, taken };
};

/// Walks the data into the matrix: two columns at a time from the right,
/// upwards then downwards, skipping the column the vertical timing pattern
/// occupies and every module a function pattern already holds.
const place = (
  size: number,
  modules: boolean[][],
  taken: boolean[][],
  data: Uint8Array,
) => {
  let bit = 0;
  for (let right = size - 1; right >= 1; right -= 2) {
    if (right === 6) right = 5;
    for (let vertical = 0; vertical < size; vertical += 1) {
      const upwards = ((size - 1 - right) & 2) === 0;
      const row = upwards ? size - 1 - vertical : vertical;
      for (const col of [right, right - 1]) {
        if (taken[row][col]) continue;
        // Past the end of the data are the remainder bits, which are zero.
        const index = bit >> 3;
        modules[row][col] = index < data.length && ((data[index] >> (7 - (bit & 7))) & 1) === 1;
        bit += 1;
      }
    }
  }
};

/// Whether a module is flipped by the given mask. The eight conditions are
/// the standard's, in its order.
const masked = (mask: number, row: number, col: number): boolean => {
  switch (mask) {
    case 0: return (row + col) % 2 === 0;
    case 1: return row % 2 === 0;
    case 2: return col % 3 === 0;
    case 3: return (row + col) % 3 === 0;
    case 4: return (Math.floor(row / 2) + Math.floor(col / 3)) % 2 === 0;
    case 5: return ((row * col) % 2) + ((row * col) % 3) === 0;
    case 6: return (((row * col) % 2) + ((row * col) % 3)) % 2 === 0;
    default: return (((row + col) % 2) + ((row * col) % 3)) % 2 === 0;
  }
};

/// Writes both copies of the format information for a mask.
const writeFormat = (size: number, modules: boolean[][], mask: number) => {
  let bits = (ECC_M_BITS << 3) | mask;
  let rest = bits;
  for (let i = 0; i < 10; i += 1) rest = (rest << 1) ^ ((rest >> 9) * 0x537);
  bits = ((bits << 10) | rest) ^ 0x5412;

  // Both copies are written most significant bit first, and both start beside
  // the top-left finder: the first wraps around it, along row 8 and then up
  // column 8; the second runs up column 8 from the bottom and on along row 8
  // to the right edge. The second stops short of the dark module, which sits
  // in that column and is not part of the format.
  const bit = (i: number) => ((bits >> (14 - i)) & 1) === 1;

  for (let i = 0; i <= 5; i += 1) modules[8][i] = bit(i);
  modules[8][7] = bit(6);
  modules[8][8] = bit(7);
  modules[7][8] = bit(8);
  for (let i = 9; i < 15; i += 1) modules[14 - i][8] = bit(i);

  for (let i = 0; i < 7; i += 1) modules[size - 1 - i][8] = bit(i);
  for (let i = 7; i < 15; i += 1) modules[8][size - 15 + i] = bit(i);
};

/// How bad a masked matrix is to read, by the standard's four rules. The mask
/// with the lowest score wins.
const penalty = (size: number, modules: boolean[][]): number => {
  let score = 0;

  // Rule 1: runs of five or more of one colour, in both directions.
  for (let i = 0; i < size; i += 1) {
    for (const row of [true, false]) {
      let run = 1;
      for (let j = 1; j < size; j += 1) {
        const here = row ? modules[i][j] : modules[j][i];
        const before = row ? modules[i][j - 1] : modules[j - 1][i];
        if (here === before) {
          run += 1;
          if (run === 5) score += 3;
          else if (run > 5) score += 1;
        } else {
          run = 1;
        }
      }
    }
  }

  // Rule 2: every block of two by two in one colour.
  for (let r = 0; r < size - 1; r += 1) {
    for (let c = 0; c < size - 1; c += 1) {
      const m = modules[r][c];
      if (m === modules[r][c + 1] && m === modules[r + 1][c] && m === modules[r + 1][c + 1]) {
        score += 3;
      }
    }
  }

  // Rule 3: the finder-like pattern, which a reader could mistake for one.
  // Dark-light-dark-dark-dark-light-dark with four light modules on either
  // side of it — and the edge of the symbol counts as light, because what
  // lies beyond it is the quiet zone. A run with fewer than four modules to
  // one side qualifies if every one of them is light.
  const needle = [true, false, true, true, true, false, true];
  const at = (line: boolean[], from: number) =>
    from + needle.length <= line.length &&
    needle.every((want, i) => line[from + i] === want);
  const allLight = (line: boolean[], from: number, to: number) => {
    for (let i = Math.max(from, 0); i < Math.min(to, line.length); i += 1) {
      if (line[i]) return false;
    }
    return true;
  };
  const finderish = (line: boolean[]): number => {
    let count = 0;
    for (let i = 0; i + needle.length <= size; ) {
      if (!at(line, i)) {
        i += 1;
        continue;
      }
      if (i === 0 || i === size - 7 || allLight(line, i - 4, i) ||
          allLight(line, i + 7, i + 11)) {
        count += 40;
        i += 7;
      } else {
        // The three dark modules in the middle are the earliest place a
        // second pattern could start, so there is no point looking before
        // them.
        i += 4;
      }
    }
    return count;
  };
  for (let i = 0; i < size; i += 1) {
    score += finderish(modules[i]);
    score += finderish(modules.map((line) => line[i]));
  }

  // Rule 4: how far the proportion of dark modules strays from half.
  let dark = 0;
  for (const line of modules) for (const m of line) if (m) dark += 1;
  const off = Math.abs((dark * 100) / (size * size) - 50);
  score += Math.floor(off / 5) * 10;

  return score;
};

/// Encodes `text` as a QR code: version M, byte mode, the smallest version
/// that holds it and the mask that scores best.
///
/// Throws if the text is longer than a version 9 code holds — see the note at
/// the top of this file about why that is the right answer and not a limit to
/// work around.
export const encodeQr = (text: string): QrCode => {
  const { version, data } = codewords(text);

  let best: { modules: boolean[][]; score: number } | null = null;
  for (let mask = 0; mask < 8; mask += 1) {
    const { size, modules, taken } = skeleton(version);
    place(size, modules, taken, data);
    for (let r = 0; r < size; r += 1) {
      for (let c = 0; c < size; c += 1) {
        if (!taken[r][c] && masked(mask, r, c)) modules[r][c] = !modules[r][c];
      }
    }
    // Scored before the format information goes in, which the standard is
    // explicit about: the mask is chosen for the encoding region, and the
    // format modules are the same fifteen positions whatever mask wins.
    const score = penalty(size, modules);
    writeFormat(size, modules, mask);
    if (best === null || score < best.score) best = { modules, score };
  }

  const modules = best!.modules;
  return { size: modules.length, modules };
};

/// The code as one SVG path: every dark module a square, in one `d`. One path
/// rather than a few hundred `<rect>`s, because this is redrawn whenever the
/// address changes and a phone browser is one of the things drawing it.
export const qrPath = (code: QrCode): string => {
  const parts: string[] = [];
  for (let r = 0; r < code.size; r += 1) {
    for (let c = 0; c < code.size; c += 1) {
      if (code.modules[r][c]) parts.push(`M${c} ${r}h1v1h-1z`);
    }
  }
  return parts.join("");
};
