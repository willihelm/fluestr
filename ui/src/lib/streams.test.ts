// The rules of the channel setup, asserted rather than argued about.

import { describe, expect, it } from "vitest";
import {
  applyLanguage,
  clampGain,
  toConfig,
  mutedChannels,
  newChannel,
  usedLanguages,
  validate,
  DEFAULT_LANGUAGE,
  type Channel,
} from "./streams";
import { GAIN_DB_MAX, GAIN_DB_MIN } from "./bridge";

const channel = (device: number, label = `Channel ${device + 1}`, language = "de"): Channel => ({
  key: device,
  channel: device,
  label,
  language,
  color: "#888888",
  gainDb: 0,
});

describe("validate", () => {
  it("starts a setup that reaches past the device, because those channels mute", () => {
    // The case this exists for: eight channels patched for the desk, plugged
    // into a two-input interface. Refusing to start would mean rebuilding the
    // setup for the small interface and again for the desk.
    const channels = [channel(0), channel(1), channel(4), channel(7)];

    expect(validate(channels, "Scarlett 2i2", 2)).toBeNull();
  });

  it("refuses only when nothing is left to listen to", () => {
    const channels = [channel(4), channel(7)];

    expect(validate(channels, "Scarlett 2i2", 2)).toBe(
      "None of the 2 channels exists on this device; it has 2.",
    );
  });

  it("names the device without input channels rather than counting to zero", () => {
    expect(validate([channel(0)], "Some output", 0)).toBe(
      "The selected device has no input channels.",
    );
  });

  it("tells a device that is not plugged in from one without inputs", () => {
    // A stored profile names the interface of the production, which is
    // regularly not the one connected to the laptop it is opened on. One of
    // the two is solved with a cable and the other with a different device, so
    // they must not read the same.
    expect(validate([channel(0)], "MADIface", 0, false)).toBe("MADIface is not connected.");
    expect(validate([channel(0)], "Some output", 0, true)).toBe(
      "The selected device has no input channels.",
    );
  });

  it("still insists on a device, a channel and a language", () => {
    expect(validate([channel(0)], "", 2)).toBe("No input device selected.");
    expect(validate([], "Scarlett 2i2", 2)).toBe("Add at least one channel.");
    expect(validate([{ ...channel(0), language: " " }], "Scarlett 2i2", 2)).toBe(
      "Channel 1 has no language.",
    );
  });
});

describe("mutedChannels", () => {
  it("answers in stream ids, which is what the meters and the events key on", () => {
    // Positions 1 and 3, not device channels 5 and 8: a muted row keeps its
    // place in the list, so the ids of the rows after it do not move.
    const channels = [channel(0), channel(4), channel(1), channel(7)];

    expect(mutedChannels(channels, 2)).toEqual([1, 3]);
  });

  it("mutes nothing when the device covers every channel", () => {
    expect(mutedChannels([channel(0), channel(7)], 8)).toEqual([]);
  });
});

describe("newChannel", () => {
  it("takes the next free channel even beyond what the device offers", () => {
    // Otherwise the setup could never be built larger than the interface
    // currently plugged in, which is the whole point of muting.
    const existing = [channel(0), channel(1)];

    expect(newChannel(existing).channel).toBe(2);
  });

  it("fills a gap left by a removed row", () => {
    expect(newChannel([channel(0), channel(2)]).channel).toBe(1);
  });

  it("is born on the profile's language, not on a fixed one", () => {
    // The point of the picker above the list: a production in English adds
    // eight channels without touching the language once.
    expect(newChannel([], "en").language).toBe("en");
    expect(newChannel([]).language).toBe(DEFAULT_LANGUAGE);
  });
});

describe("applyLanguage", () => {
  it("puts every channel on one language", () => {
    const channels = [channel(0, "Regie", "de"), channel(1, "FOH", "en")];

    expect(applyLanguage(channels, "en").map((c) => c.language)).toEqual(["en", "en"]);
  });

  it("leaves the rows that already match untouched", () => {
    // Identity, not equality: these are the objects the Settings page is
    // bound to, and replacing one the operator is editing would cost them
    // the field they are in.
    const channels = [channel(0, "Regie", "en"), channel(1, "FOH", "de")];
    const applied = applyLanguage(channels, "en");

    expect(applied[0]).toBe(channels[0]);
    expect(applied[1]).not.toBe(channels[1]);
  });

  it("changes nothing else about a row", () => {
    const channels = [channel(3, "Licht", "de")];
    const [applied] = applyLanguage(channels, "fr");

    expect(applied).toEqual({ ...channels[0], language: "fr" });
  });
});

describe("usedLanguages", () => {
  it("names each language once, in the order the channels are in", () => {
    const channels = [
      channel(0, "Regie", "de"),
      channel(1, "FOH", "en"),
      channel(2, "Licht", "de"),
    ];

    expect(usedLanguages(channels)).toEqual(["de", "en"]);
  });

  it("is empty for a setup with no channels", () => {
    expect(usedLanguages([])).toEqual([]);
  });
});

describe("clampGain", () => {
  it("keeps a trim inside the range the backend accepts", () => {
    expect(clampGain(6)).toBe(6);
    expect(clampGain(GAIN_DB_MAX + 30)).toBe(GAIN_DB_MAX);
    expect(clampGain(GAIN_DB_MIN - 30)).toBe(GAIN_DB_MIN);
  });

  it("is whole decibels, because that is what the knob and the file hold", () => {
    expect(clampGain(3.4)).toBe(3);
    expect(clampGain(-3.6)).toBe(-4);
  });

  // A number box that has been cleared arrives as NaN, and a trim of NaN is a
  // channel scaled to silence rather than one left alone.
  it("reads a value that is not a number as no trim at all", () => {
    expect(clampGain(Number.NaN)).toBe(0);
  });
});

describe("toConfig", () => {
  it("carries each channel's trim to the backend", () => {
    const channels = [channel(0, "Regie"), { ...channel(1, "Licht"), gainDb: 9 }];

    expect(toConfig(channels).map((c) => c.gainDb)).toEqual([0, 9]);
  });

  it("clamps a trim a hand-edited file put out of range", () => {
    const channels = [{ ...channel(0, "Regie"), gainDb: 400 }];

    expect(toConfig(channels)[0].gainDb).toBe(GAIN_DB_MAX);
  });
});
