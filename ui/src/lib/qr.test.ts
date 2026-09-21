// The QR encoder, against codes somebody else made (P16).
//
// This is the one piece of the frontend that cannot be checked by looking at
// it: a code with a wrong module somewhere in the middle looks exactly like a
// right one, and the failure shows up as a phone that will not scan while the
// operator is standing in front of a rack. So it is compared module for
// module with the output of `segno`, an independent implementation of the
// same standard.
//
// `qr.fixture.json` was generated with, in short:
//
//     segno.make(text, error='m', mode='byte', micro=False,
//                encoding='utf-8', boost_error=False).matrix
//
// The options pin it to what `encodeQr` does — level M, byte mode, no micro
// codes, and no quiet raising of the error-correction level when a shorter
// string leaves room for it. The cases include one string at the exact
// capacity of every version from 1 to 9, so every block structure in the
// table is covered, and one with multi-byte characters.
//
// One correction was made to segno while generating it: its
// `write_padding_bits` adds a whole zero codeword when the bit stream already
// ends on a codeword boundary, which in byte mode it always does. The symbol
// stays readable — a decoder stops at the terminator — but it is a codeword
// of capacity spent on nothing, and it is not what the standard asks for, so
// the fixture was generated with that guarded by `and length % 8`.
//
// Every matrix here, and every matrix `encodeQr` produces for these strings,
// was additionally read back with `zxing-cpp` and gave the original text.

import { describe, expect, it } from "vitest";
import { encodeQr, qrPath } from "./qr";
import fixture from "./qr.fixture.json";

const render = (rows: boolean[][]): string[] =>
  rows.map((row) => row.map((dark) => (dark ? "1" : "0")).join(""));

describe("encodeQr", () => {
  for (const sample of fixture) {
    const name = sample.text.length > 24 ? `${sample.text.slice(0, 24)}… (${sample.text.length})` : sample.text;
    it(`matches the reference for ${name}`, () => {
      const code = encodeQr(sample.text);
      expect(code.size).toBe(sample.rows.length);
      expect(code.size).toBe(sample.version * 4 + 17);
      expect(render(code.modules)).toEqual(sample.rows);
    });
  }

  it("refuses more than a version 9 code holds", () => {
    expect(() => encodeQr("x".repeat(181))).toThrow(/too long/);
  });

  it("takes the whole of what a version 9 code holds", () => {
    expect(encodeQr("x".repeat(180)).size).toBe(53);
  });
});

describe("qrPath", () => {
  it("draws one square per dark module, in module units", () => {
    const code = encodeQr("http://192.168.1.23:8420");
    const dark = code.modules.flat().filter(Boolean).length;
    const path = qrPath(code);
    expect(path.match(/M/g)?.length).toBe(dark);
    // The top-left module of the top-left finder is always dark, and the path
    // is laid out in module coordinates so the SVG can scale it freely.
    expect(path.startsWith("M0 0h1v1h-1z")).toBe(true);
  });
});
