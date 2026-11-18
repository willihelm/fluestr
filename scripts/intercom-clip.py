#!/usr/bin/env python3
"""Writes a two-channel test recording of intercom traffic, for playing in as
an audio file (Settings -> Audio -> "Play in an audio file").

Channel 1 is a German stage manager, channel 2 an English lighting operator.
Each opens with one long line and then gives short calls: a one-second "Go!"
is where models most often guess the language wrong. The calls never overlap,
and a faint noise floor stands in for an open beltpack.

macOS only: the voices come from `say`, so the audio is generated locally
and not committed (only the generating code belongs in the repository).

    python3 scripts/intercom-clip.py [out.wav]

Default output: .scratch/fixtures/intercom-de-en.wav (48 kHz, 16-bit, 2 ch).
The spoken text is German and English on purpose: it is speech data, not code.
"""

import array
import os
import random
import subprocess
import sys
import tempfile
import wave

RATE = 48_000

# (seconds, channel, voice, text)
LINES = [
    (0.5, 0, "Anna", "Achtung, hier spricht die Regie. Wir beginnen gleich mit dem zweiten Akt, bitte alle auf ihre Positionen."),
    (8.5, 1, "Daniel", "Lighting here, all desks are ready and the house lights are on the board."),
    (14.5, 0, "Anna", "Licht, Standby."),
    (16.5, 1, "Daniel", "Standing by."),
    (18.5, 0, "Anna", "Standby Cue zwölf."),
    (21.0, 0, "Anna", "Go!"),
    (23.0, 1, "Daniel", "Go."),
    (25.0, 0, "Anna", "Vorhang auf."),
    (27.5, 0, "Anna", "Bühne frei."),
    (30.0, 1, "Daniel", "Copy that."),
    (32.0, 0, "Anna", "Regie hört."),
    (34.5, 0, "Anna", "Kamera zwei, bitte."),
    (37.0, 1, "Daniel", "Fade out now."),
    (39.5, 0, "Anna", "Danke, fertig."),
]
TAIL_S = 3.0
NOISE = 0.0008  # about -62 dBFS


def speak(voice, text, workdir):
    aiff = os.path.join(workdir, "line.aiff")
    wav = os.path.join(workdir, "line.wav")
    subprocess.run(["say", "-v", voice, "-o", aiff, text], check=True)
    subprocess.run(
        ["afconvert", "-f", "WAVE", "-d", f"LEI16@{RATE}", "-c", "1", aiff, wav],
        check=True,
    )
    with wave.open(wav) as w:
        return array.array("h", w.readframes(w.getnframes()))


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else ".scratch/fixtures/intercom-de-en.wav"
    os.makedirs(os.path.dirname(out) or ".", exist_ok=True)
    with tempfile.TemporaryDirectory() as workdir:
        spoken = [(at, ch, speak(voice, text, workdir)) for at, ch, voice, text in LINES]

    end = max(int(at * RATE) + len(pcm) for at, _, pcm in spoken) + int(TAIL_S * RATE)
    rng = random.Random(7)
    channels = [[rng.gauss(0.0, NOISE) for _ in range(end)] for _ in range(2)]
    for at, ch, pcm in spoken:
        start = int(at * RATE)
        track = channels[ch]
        for i, sample in enumerate(pcm):
            track[start + i] += sample / 32768.0

    frames = array.array("h")
    for left, right in zip(*channels):
        frames.append(max(-32768, min(32767, int(left * 32767))))
        frames.append(max(-32768, min(32767, int(right * 32767))))
    with wave.open(out, "wb") as w:
        w.setnchannels(2)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(frames.tobytes())
    print(f"{out}: {end / RATE:.1f} s, 2 channels at {RATE} Hz")
    for at, ch, voice, text in LINES:
        print(f"  {at:5.1f} s  ch {ch + 1}  {text}")


if __name__ == "__main__":
    main()
