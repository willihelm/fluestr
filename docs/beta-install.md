# Installing the Fluestr beta

This build is **not signed with an Apple Developer ID**, so macOS will block it
the first time and you have to let it through by hand. That is expected, and
the steps below are the whole of it. Everything else is a normal install.

## You need

- A Mac with **Apple Silicon** (M1 or newer). Intel Macs are not supported.
- **macOS 13** or newer.
- An internet connection **for the first start** — the app downloads its speech
  model then, about 190 MB. After that it works offline, and nothing you say
  ever leaves the machine.
- The audio interface you want to listen to, plugged in.

## Install

1. Open `Fluestr_<version>_aarch64.dmg` and drag **Fluestr** onto the
   **Applications** folder.
2. Eject the disk image, then open Fluestr from Applications.
3. macOS refuses it: *"Apple could not verify 'Fluestr' is free of malware …"*.
   Click **Done**. This is the expected step, not a failure.
4. Open **System Settings → Privacy & Security** and scroll down to
   **Security**. There is a line saying *"Fluestr was blocked to protect your
   Mac."* — click **Open Anyway** and authenticate.
5. Open Fluestr again and confirm **Open** in the last dialog.

That is once per install. Do not bother trying to Control-click the app and
choose Open — macOS removed that shortcut, it no longer works.

From version 0.2.0 on, Fluestr updates itself: when a newer release is out, a
button appears next to the status dot in the header, and installing from there
needs none of the steps above. macOS may ask for microphone access again after
an update — allow it as on the first start.

<details>
<summary>If you would rather use the Terminal</summary>

```sh
xattr -d -r com.apple.quarantine /Applications/Fluestr.app
```

Same effect as steps 3–5, and it has to be repeated for each new build.
</details>

## Why the warning

Apple only vouches for apps signed with a paid Developer ID and sent through
their notarisation service. This beta has neither yet, so macOS cannot tell you
who built it and says so in the strongest words it has. The app **is**
cryptographically signed and macOS verifies that signature — what is missing is
the identity behind it, not the integrity of the file. Once Fluestr has a
Developer ID, this whole section disappears and it installs by double-click.

If your Mac is managed by an employer, the **Open Anyway** button may be
disabled by policy. In that case this build cannot be installed at all — tell
me and wait for a signed one rather than fighting it.

## First start

The setup guide opens by itself. It downloads the speech model, fetches the
speech detector, and walks you through the audio interface, its channels and
the groups. It can be skipped and is at the foot of the Settings rail
afterwards.

- **The model download is about 190 MB** and the progress bar is honest — let
  it finish.
- **The very first transcription takes about eight seconds**, because macOS
  compiles the graphics shaders once. Every one after that is a fraction of a
  second. It is not hung.
- macOS asks for **microphone access**. You have to allow it: without it the
  app receives silence and every channel simply stays empty, which looks like a
  bug rather than a denied permission.

Then pick a device, pick a channel, press **Start**.

## Updating to the next beta

Replace the app in Applications and **repeat the unblock steps above** — each
new build is a new file to macOS, so the warning comes back.

macOS may also ask for microphone access again after an update. Allow it. If a
channel stays silent after an update, that is the first thing to check:
**System Settings → Privacy & Security → Microphone**.

## What is worth reporting

This is the first build that has ever been installed on a machine other than
the one it was written on, so the interesting failures are the boring ones:

- Anything in the **setup guide** that is wrong, stuck or confusing — that path
  has never been walked on a fresh Mac before you.
- An audio interface whose **channels are named or counted wrong**, or that
  does not appear at all.
- Channels that stay **empty** while people are clearly talking.
- Transcripts that are **wrong in a way that matters** — names, cues, numbers.
  A rough transcript is expected; a confidently wrong cue number is not.
- Anything that made you **stop trusting it** during a show.

Rough notes are fine. When in doubt, say what you did, what you expected, and
what happened instead.
