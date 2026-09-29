//! The configuration file, and the profiles in it (P12).
//!
//! Everything the operator builds on the Settings page — the channel map with
//! its labels, languages and colours, the groups and their memberships, the
//! active vocabulary — used to live exactly as long as the window did. Nobody
//! remaps eight channels and rebuilds four views before a show, so it is
//! stored here and read back at startup.
//!
//! **Profiles** are the second half of that. One engineer works several
//! productions on the same machine, and each one has its own patch: "Oper
//! grosses Haus" is eight channels in five groups, "Probebühne" is three. A
//! profile is one such setup, they are switched on the Settings page, and the
//! one last used comes back at startup.
//!
//! What is *not* in a profile is the vocabulary: the plan puts it at the top
//! level on purpose (P12, P15). The jargon of a house — its cue names, its
//! departments — outlives any one patch, and copying it into every profile
//! would be the same drift the single session-wide prompt was introduced to
//! avoid.
//!
//! The file is JSON rather than the SQLite database P10 will bring, and that
//! is deliberate: configuration is read once at startup and written when a
//! human edits something, which is nothing like the write load transcripts
//! have. It is also the one file an operator may have to fix by hand or send
//! to someone else, and a text file can be fixed by hand.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::rules;
use crate::webhook::Webhook;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Schema version of the configuration file.
///
/// Written on every save and checked on every load. A file from a *newer*
/// version is not silently reinterpreted — see [`load`]. An *older* one is
/// migrated on the way in — see [`Config::migrate`], which is what turned the
/// alert list and the two flash flags of version 1 into the rules of version
/// 2.
/// 3 put the shoutbox's room levels in dB SPL throughout (P26) — see
/// [`Shoutbox::lift_to_spl`].
pub const VERSION: u32 = 3;

/// The file inside the app's configuration directory.
pub const FILE_NAME: &str = "config.json";

/// One intercom channel as it is stored.
///
/// Close to, but not the same as, [`crate::session::StreamConfig`]: the colour
/// is part of a channel's configuration rather than a rendering detail — a
/// channel is recognised by its colour on the meter strip and on every chat
/// line long before anyone reads its label — and it means nothing to the
/// capture pipeline, so it lives here and not in what starts a session.
///
/// The channel's UI key is deliberately absent. It is a counter handed out
/// while the window is open; it identifies a row to Svelte and would be noise
/// in a file a human may have to read. Group membership is stored as positions
/// instead — see [`StoredGroup::members`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "config.ts")]
pub struct StoredChannel {
    /// Zero-based channel index on the multichannel device, as the audio API
    /// counts them. The UI shows it one-based and converts at its own
    /// boundary.
    pub channel: u16,
    pub label: String,
    /// ISO 639-1, never a hard-coded enum: the language list is extensible by
    /// requirement.
    pub language: String,
    /// CSS colour. Preselected from the palette when the channel is created,
    /// then an explicit user-editable value.
    pub color: String,
    /// Input trim in whole decibels, as the knob on the meter strip sets it.
    ///
    /// Stored with the channel and not with the device: it compensates for how
    /// hot *this* intercom position is patched, which is a property of the
    /// production's patch list and therefore of the profile. Carried into the
    /// session as [`crate::session::StreamConfig::gain_db`].
    ///
    /// `#[serde(default)]` because a file written before the trim existed has
    /// no such field, and zero — unity — is exactly what those channels did.
    #[serde(default)]
    pub gain_db: i32,
    /// Whether the channel is muted — metered but not transcribed, as the M
    /// beside its meter sets it.
    ///
    /// Stored, though it is pressed during a show rather than during setup: a
    /// position that is muted is usually muted for a reason that outlives the
    /// evening — a beltpack in a noisy workshop, a channel patched but not
    /// used by this production — and an operator who muted it once should not
    /// have to find it again after every restart. Carried into the session as
    /// [`crate::session::StreamConfig::muted`].
    ///
    /// `#[serde(default)]` for the trim's reason: a file written before mute
    /// existed describes channels that were all unmuted.
    #[serde(default)]
    pub muted: bool,
    /// Text that looks like this channel — its terms and names, written the
    /// way they should come out — put into whisper's prompt for this channel
    /// only, after the global vocabulary (P25).
    ///
    /// Not an instruction: whisper reads its prompt as what was said before
    /// the segment, and a sentence telling it what to do is at best ignored
    /// and at worst continued on a quiet channel. On the channel rather than
    /// beside the vocabulary because it describes a position in the patch,
    /// and the patch is the profile's. Carried into the session as
    /// [`crate::session::StreamConfig::description`].
    ///
    /// `#[serde(default)]`: a file written before it existed has channels
    /// with nothing to describe them, which is an empty one.
    #[serde(default)]
    pub description: String,
}

/// A named view onto some of the channels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "config.ts")]
pub struct StoredGroup {
    pub name: String,
    /// Positions in [`Profile::channels`], not the UI's channel keys and not
    /// stream ids.
    ///
    /// At runtime membership is keyed on the channel's own key, precisely so
    /// that deleting a channel does not re-point every group below it. That
    /// argument is about a list being *edited*; a saved file is a snapshot in
    /// which positions and keys say the same thing, and positions are the one
    /// of the two that still means something after a restart. The translation
    /// happens in the UI, in one place, on the way in and on the way out.
    ///
    /// A position past the end of the channel list is dropped on load rather
    /// than refused: a hand-edited file should cost a group its member, not
    /// the operator their whole setup.
    pub members: Vec<usize>,
    /// **Version 1 only.** Whether a view of this group flooded its window on
    /// every message (P17).
    ///
    /// Read, never written: [`Config::migrate`] turns it into a rule with a
    /// group trigger and the flash action, which is the same thing said once
    /// in the one place every reaction is now written. Kept as a field so that
    /// a file from before P19 can still be read; `skip_serializing` is what
    /// makes the migration a one-way trip rather than a flag that comes back
    /// beside the rule it became.
    #[serde(default, skip_serializing)]
    #[ts(skip)]
    pub flash: bool,
    /// Whether this group's detached window (P7) stays in front of other
    /// windows.
    ///
    /// Set on that window itself, by the pin in its header, and stored here
    /// only so the next window this group opens comes up the same way. Per
    /// group rather than a property of every group window — the stage
    /// manager's view is the one that has to stay visible over whatever else
    /// is being worked in, and the general talkback covering it would be the
    /// opposite of useful.
    ///
    /// `#[serde(default)]` — off — because a file written before this existed
    /// has no such field, and a setup that suddenly opens windows nothing can
    /// be put in front of is not what its absence meant.
    #[serde(default)]
    pub on_top: bool,
}

/// One rule: a trigger, and what it does (P19).
///
/// Stored as the operator wrote it — words as words, channels and views as
/// positions — and *compiled* elsewhere: the frontend resolves the channels
/// into stream ids and hands [`crate::rules::CompiledRule`] to the backend.
/// This is the form a human can read and edit in the file, which is the same
/// reason the alert terms it replaces were a list of strings rather than a
/// serialised matcher.
///
/// **Channels and views are not two spellings of one thing.** The trigger is
/// a conjunction of two questions — these words (or any text) said on any of
/// these *channels* (or on any channel) — and a channel is an input. A
/// *view* is a window, and it answers a question the trigger cannot: which
/// window the flash is for. An empty channel list is every channel, which is
/// how "flash on every message" is written; an empty word list is any text,
/// which is how "anything at all on channel 3" is; an empty view list is
/// every window that shows the channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "config.ts")]
pub struct StoredRule {
    /// What the operator calls it. Free-form and allowed to be empty: the
    /// editor shows the trigger anyway, and a name nobody needed is a field
    /// that would have to be invented to save a rule.
    pub name: String,
    /// Off keeps a rule without deleting it — the show where the cue is not
    /// wanted is not the show where the rule should be rewritten from memory.
    #[serde(default = "enabled")]
    pub enabled: bool,
    /// Words and phrases, matched whole-word, case-insensitively and
    /// NFC-normalised — P17's matcher, unchanged. Empty is no word condition.
    #[serde(default)]
    pub words: Vec<String>,
    /// Positions in [`Profile::channels`], like [`StoredGroup::members`] and
    /// for the same reason: a saved file is a snapshot, and a position is the
    /// one of position and key that still means something after a restart. Out
    /// of range is dropped on load.
    #[serde(default)]
    pub channels: Vec<usize>,
    /// Positions in [`Profile::groups`]: the windows the *flash* is for.
    ///
    /// A group is a view, so naming one here does not narrow *when* the rule
    /// fires — [`StoredRule::channels`] does that — but *who is told*. Empty
    /// is every window that shows the channel, which is what the flash did
    /// before it could be addressed at all.
    #[serde(default)]
    pub views: Vec<usize>,
    pub actions: Vec<crate::rules::Action>,
    /// The request [`crate::rules::Action::Webhook`] makes (P19), or `None`
    /// for a rule that makes none.
    ///
    /// Stored with the rule like everything else on it, and — unlike the rest
    /// of the rule — **written only on the machine**: the rule editor served
    /// to the LAN remote has no URL box, because an arbitrary destination
    /// writable from an unauthenticated browser form is a different
    /// proposition from the read-mostly remote P16 was decided for. See
    /// `ui/src/lib/RuleEditor.svelte`.
    #[serde(default)]
    pub webhook: Option<crate::webhook::Webhook>,
    /// At most one firing per this many milliseconds. Zero — the default — is
    /// unthrottled.
    ///
    /// Per rule and not per action, and stored rather than hard-coded, because
    /// a channel trigger on a busy talkback fires several times a minute:
    /// harmless for a highlight, and exactly the flood P19's later actions
    /// must not turn into a stream of MIDI. A rule inside its interval does
    /// nothing at all — see [`crate::rules`].
    #[serde(default)]
    pub min_interval_ms: u64,
}

/// What a rule without a stored `enabled` is: on. A rule that was written is a
/// rule that was wanted, and a file hand-edited to add one should not need the
/// field that says so.
fn enabled() -> bool {
    true
}

/// One production's setup: which interface, which channels, which views.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "config.ts", rename = "StoredProfile")]
pub struct Profile {
    /// Free-form and shown in the switcher: "Oper grosses Haus", "Probebühne".
    pub name: String,
    /// Device name, as `list_devices` returns it.
    ///
    /// Kept beside the id rather than replaced by it, and for two reasons.
    /// A device that is not plugged in has to be *named* — "Scarlett 18i20 is
    /// not connected" is a sentence an operator can act on, a UID is not. And
    /// a file written before ids existed holds only this, which is what
    /// [`Profile::device_id`] is then filled in from.
    ///
    /// A stored device that is not currently connected is kept and marked in
    /// the UI, never silently replaced with whatever is plugged in.
    pub device: String,
    /// The device's stable id — on macOS its CoreAudio UID.
    ///
    /// This is what the profile is actually keyed on: it survives a rename
    /// and it distinguishes two units of the same model, neither of which the
    /// name does. See `audio_core::DeviceId`.
    ///
    /// Empty means "not known yet": a file written before this field existed,
    /// or a device the platform would not identify. The UI fills it in the
    /// first time it sees the named device connected, so an existing setup
    /// gains a stable id by being used rather than by being re-entered.
    #[serde(default)]
    pub device_id: String,
    /// The language a channel added to this profile starts on, and what the
    /// picker above the channel list shows.
    ///
    /// Per profile rather than per machine: a house may run a German main
    /// stage and an English touring production, and the thing this presets —
    /// the channel map — belongs to the profile too. Every channel still
    /// carries its own language; this only decides what a new one is born
    /// with.
    ///
    /// `#[serde(default)]` because a file written before this existed has no
    /// such field, and its profiles must keep loading.
    #[serde(default = "default_language")]
    pub language: String,
    pub channels: Vec<StoredChannel>,
    pub groups: Vec<StoredGroup>,
    /// **Version 1 only.** Whether the all-channels view flooded its window on
    /// every message (P17).
    ///
    /// Read, never written, like [`StoredGroup::flash`]: it becomes a rule
    /// with no trigger at all — every message, on every channel — which is
    /// what the flag said and what P19 can now write down.
    #[serde(default, skip_serializing)]
    #[ts(skip)]
    pub all_channels_flash: bool,
    /// The rules of this profile: what fires, when (P19).
    ///
    /// On the profile rather than beside it, unlike the vocabulary and unlike
    /// the alert list it replaces. A trigger names channels and groups, and
    /// neither exists outside a profile: a shared list could hold "anything on
    /// channel 3", and channel 3 is a different channel in the next
    /// production. The cost is that a word rule an operator wants everywhere
    /// is written in each profile — which is why the migration copies the old
    /// global alert terms into every one of them rather than into the first.
    ///
    /// `#[serde(default)]` because a file written before P19 has no such
    /// field; [`Config::migrate`] is what fills it in for those.
    #[serde(default)]
    pub rules: Vec<StoredRule>,
    /// Where audio leaves the app (P22, P26): the output replays are heard
    /// on, and the shoutbox that follows the room.
    ///
    /// On the profile rather than beside it, because both are patched on the
    /// interface the profile names: the shoutbox's program is one of its
    /// intercom channels, and the headphones are on its outputs. A patch
    /// carried to the next production carries these with it.
    ///
    /// `#[serde(default)]` — no shoutbox, replays on the system output —
    /// because a file written before these existed describes an app that
    /// never sent audio anywhere.
    #[serde(default)]
    pub outputs: Outputs,
}

/// One output of a device, as a route is stored (P22, P26).
///
/// Name and id together, like [`Profile::device`]: the id is what is opened,
/// the name is how a device that is not plugged in is still named.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "config.ts", rename = "StoredOutputRoute")]
pub struct OutputRoute {
    /// Empty for the listen output means the system's default output; for
    /// the shoutbox it means "not patched yet".
    #[serde(default)]
    pub device: String,
    #[serde(default)]
    pub device_id: String,
    /// Zero-based output channels the mono signal goes to: one for a
    /// speaker, two for a pair of headphones. Empty is the first two of the
    /// device, or its only one.
    #[serde(default)]
    pub channels: Vec<u16>,
}

/// One input channel of a device, as a route is stored (P26).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "config.ts", rename = "StoredInputRoute")]
pub struct InputRoute {
    /// Empty means "not patched yet".
    #[serde(default)]
    pub device: String,
    #[serde(default)]
    pub device_id: String,
    /// Zero-based channel on that device.
    #[serde(default)]
    pub channel: u16,
}

/// How a bubble's audio is heard again (P22).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "config.ts", rename = "StoredListen")]
pub struct Listen {
    /// The *listen* output — typically headphones — so a replay reaches the
    /// operator's ears and nobody else's. An empty device is the system
    /// output, which is where replays go until somebody chooses.
    #[serde(default)]
    pub output: OutputRoute,
    /// How many minutes of *speech* are kept for replaying, across every
    /// channel. 0 keeps nothing. Behind it stands a hard byte ceiling —
    /// see `replay::MAX_BYTES`.
    #[serde(default = "default_keep_minutes")]
    pub keep_minutes: u32,
}

fn default_keep_minutes() -> u32 {
    10
}

impl Default for Listen {
    fn default() -> Self {
        Self {
            output: OutputRoute::default(),
            keep_minutes: default_keep_minutes(),
        }
    }
}

/// The shoutbox that follows the room (P26).
///
/// **Whole numbers throughout**, for [`Tuning`]'s reasons: the file is read
/// and repaired by hand, and `==` is how a save that changes nothing is told
/// from one that does. Levels are whole dB, the slope is percent, speeds are
/// the milliseconds ten decibels take. The one exception is the calibration,
/// which a calibrator gives to a tenth — kept as [`Tenths`], still an integer.
///
/// The room's levels — the reading and the curve's quiet threshold — are in
/// dB SPL, A-weighted, always: what an engineer thinks in. Without a
/// calibration they are an estimate, read through
/// [`Shoutbox::ESTIMATED_CALIBRATION_DB`]. The program's levels — the gate and
/// the ceiling — stay in dBFS, because they are the signal's and not the
/// room's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export, export_to = "config.ts", rename = "StoredShoutbox")]
pub struct Shoutbox {
    pub enabled: bool,
    /// The intercom channel the shoutbox plays.
    pub program: InputRoute,
    /// The measurement microphone.
    pub reference: InputRoute,
    /// The speaker. Has to be on the program's interface — one clock, or the
    /// pass-through drifts.
    pub output: OutputRoute,
    /// The averaging time of the room measurement. Short by default: it
    /// slows the gain on top of the rise and fall, and a second leaves those
    /// two in charge of how fast the shoutbox follows.
    pub window_ms: u32,
    /// Added to weighted dBFS to read dB SPL, read off an SPL meter or set
    /// with a calibrator. `None` is uncalibrated, and reads through
    /// [`Shoutbox::ESTIMATED_CALIBRATION_DB`] instead.
    #[ts(optional, type = "number | null")]
    pub calibration_db: Option<Tenths>,
    /// Below this room level, in dB SPL, nothing changes.
    pub quiet_db: i32,
    /// The gain at and below the quiet level.
    pub base_gain_db: i32,
    /// dB of gain per dB of room, in percent.
    pub slope_percent: u32,
    /// The most the curve will ask for.
    pub max_gain_db: i32,
    /// How long a rise of 10 dB takes, so a loud song is followed within
    /// seconds.
    pub rise_ms: u32,
    /// How long a fall of 10 dB takes. 0 is no limit: the averaging window
    /// alone slows the fall, which is often slow enough on its own.
    pub fall_ms: u32,
    /// Above this pre-gain peak the shoutbox counts as playing, and the room
    /// is not measured — it would be measuring itself.
    pub gate_dbfs: i32,
    /// How long it keeps counting as playing after the last word.
    pub gate_hold_ms: u32,
    /// Nothing leaves above this, whatever the curve asked for.
    pub ceiling_dbfs: i32,
    /// The operator's hand on top of the automation.
    pub trim_db: i32,
    pub muted: bool,
}

impl Default for Shoutbox {
    fn default() -> Self {
        Self {
            enabled: false,
            program: InputRoute::default(),
            reference: InputRoute::default(),
            output: OutputRoute::default(),
            window_ms: 1_000,
            calibration_db: None,
            quiet_db: 50,
            base_gain_db: -12,
            slope_percent: 70,
            max_gain_db: 0,
            rise_ms: 5_000,
            fall_ms: 5_000,
            gate_dbfs: -45,
            gate_hold_ms: 1_500,
            ceiling_dbfs: -1,
            trim_db: 0,
            muted: false,
        }
    }
}

impl Shoutbox {
    /// The ranges the page offers and the file is clamped into.
    pub const GAIN_DB: std::ops::RangeInclusive<i32> = -40..=12;
    pub const TRIM_DB: std::ops::RangeInclusive<i32> = -20..=12;
    pub const LEVEL_DB: std::ops::RangeInclusive<i32> = -120..=140;
    pub const WINDOW_MS: std::ops::RangeInclusive<u32> = 500..=30_000;
    pub const SPEED_MS: std::ops::RangeInclusive<u32> = 500..=120_000;
    /// The fall's, which may be 0 — see [`Shoutbox::fall_ms`].
    pub const FALL_MS: std::ops::RangeInclusive<u32> = 0..=120_000;
    pub const GATE_DBFS: std::ops::RangeInclusive<i32> = -80..=-10;
    pub const HOLD_MS: std::ops::RangeInclusive<u32> = 200..=10_000;
    pub const CEILING_DBFS: std::ops::RangeInclusive<i32> = -20..=0;
    pub const SLOPE_PERCENT: std::ops::RangeInclusive<u32> = 0..=200;
    pub const CALIBRATION_DB: std::ops::RangeInclusive<i32> = -200..=200;

    /// What an uncalibrated input is read through: weighted dBFS plus this is
    /// an estimate of dB SPL. Roughly right for a laptop's own mic and for a
    /// measurement mic at usual preamp gain — a quiet room at about 30, a
    /// shout at about 90 — and easily ten off either way, which is what
    /// calibrating is for. The page says the levels are estimated until then.
    pub const ESTIMATED_CALIBRATION_DB: f32 = 100.0;

    /// The offset levels are read through, calibrated or not.
    pub fn offset_db(&self) -> f32 {
        self.calibration_db
            .map_or(Self::ESTIMATED_CALIBRATION_DB, Tenths::db)
    }

    /// Moves an uncalibrated curve written before version 3 from weighted
    /// dBFS to the estimated SPL it is read in now, so it stays where it was
    /// in the room.
    ///
    /// Only a quiet threshold at or below 0: a weighted dBFS level cannot be
    /// above full scale, so anything higher was never one — it is a profile
    /// the file did not have a shoutbox for, which the default filled in, and
    /// the default is SPL already. A calibrated curve was in dB SPL all along.
    fn lift_to_spl(&mut self) {
        if self.calibration_db.is_none() && self.quiet_db <= 0 {
            self.quiet_db += Self::ESTIMATED_CALIBRATION_DB as i32;
        }
    }

    fn clamp(&mut self) {
        let clamp_i =
            |v: &mut i32, r: &std::ops::RangeInclusive<i32>| *v = (*v).clamp(*r.start(), *r.end());
        let clamp_u =
            |v: &mut u32, r: &std::ops::RangeInclusive<u32>| *v = (*v).clamp(*r.start(), *r.end());
        clamp_i(&mut self.base_gain_db, &Self::GAIN_DB);
        clamp_i(&mut self.max_gain_db, &Self::GAIN_DB);
        clamp_i(&mut self.trim_db, &Self::TRIM_DB);
        clamp_i(&mut self.quiet_db, &Self::LEVEL_DB);
        clamp_i(&mut self.gate_dbfs, &Self::GATE_DBFS);
        clamp_i(&mut self.ceiling_dbfs, &Self::CEILING_DBFS);
        clamp_u(&mut self.window_ms, &Self::WINDOW_MS);
        clamp_u(&mut self.rise_ms, &Self::SPEED_MS);
        clamp_u(&mut self.fall_ms, &Self::FALL_MS);
        clamp_u(&mut self.gate_hold_ms, &Self::HOLD_MS);
        clamp_u(&mut self.slope_percent, &Self::SLOPE_PERCENT);
        if let Some(offset) = self.calibration_db.as_mut() {
            offset.0 = offset.0.clamp(
                *Self::CALIBRATION_DB.start() * 10,
                *Self::CALIBRATION_DB.end() * 10,
            );
        }
    }

    /// The controller's view of these settings.
    pub fn control(&self) -> audio_core::shoutbox::ControlSettings {
        use audio_core::shoutbox::{ControlSettings, Curve};
        let per_second = |ms: u32| 10.0 / (ms.max(1) as f32 / 1000.0);
        ControlSettings {
            curve: Curve {
                quiet_db: self.quiet_db as f32,
                base_gain_db: self.base_gain_db as f32,
                slope: self.slope_percent as f32 / 100.0,
                max_gain_db: self.max_gain_db as f32,
            },
            rise_db_per_s: per_second(self.rise_ms),
            fall_db_per_s: match self.fall_ms {
                0 => f32::INFINITY,
                ms => per_second(ms),
            },
            window: std::time::Duration::from_millis(self.window_ms as u64),
            calibration_db: self.offset_db(),
            gate_threshold_dbfs: self.gate_dbfs as f32,
            gate_hold: std::time::Duration::from_millis(self.gate_hold_ms as u64),
        }
    }

    /// Whether `other` needs the streams reopened, rather than only new
    /// numbers handed to the running controller.
    pub fn same_streams(&self, other: &Shoutbox) -> bool {
        self.enabled == other.enabled
            && self.program == other.program
            && self.reference == other.reference
            && self.output == other.output
            && self.ceiling_dbfs == other.ceiling_dbfs
    }
}

/// A level to a tenth of a dB, stored as the integer number of tenths so the
/// file keeps comparing with `==`.
///
/// Written as the plain decimal (`117.4`), so the file reads as it always did
/// and an older whole-number value (`118`) loads unchanged.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tenths(pub i32);

impl Tenths {
    pub fn from_db(db: f64) -> Self {
        Self((db * 10.0).round() as i32)
    }

    pub fn db(self) -> f32 {
        self.0 as f32 / 10.0
    }
}

impl Serialize for Tenths {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_f64(self.0 as f64 / 10.0)
    }
}

impl<'de> Deserialize<'de> for Tenths {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let db = f64::deserialize(deserializer)?;
        if !db.is_finite() {
            return Err(serde::de::Error::custom(
                "a level has to be a finite number",
            ));
        }
        Ok(Self::from_db(db))
    }
}

/// Everything about where audio leaves the app (P22, P26).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export, export_to = "config.ts", rename = "StoredOutputs")]
pub struct Outputs {
    pub listen: Listen,
    pub shoutbox: Shoutbox,
}

/// What a profile without a stored default language falls back to.
///
/// The only place in the backend that names a language at all. It is a
/// starting point for a new channel, never a constraint: the picker is fed
/// from whisper's own list (see `list_languages`), and any of the hundred
/// entries there can be chosen.
fn default_language() -> String {
    "de".to_string()
}

/// Which end of the chat new lines arrive at (P7).
///
/// Stored rather than only chosen in a window, because it is the one thing
/// about the chat that an operator wants to be true of every window before
/// they have opened it: the default a new main window, a detached group window
/// and a browser on the LAN are all born with. A window may then be turned
/// over on its own — that override belongs to the view and is kept by it, not
/// written here.
///
/// Named after where the newest line is rather than after a sort order:
/// "descending" needs the reader to know what is being sorted by, and every
/// caller is asking the same question about the same thing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export, export_to = "config.ts", rename = "StoredSortDirection")]
pub enum SortDirection {
    /// What a chat is, and what this list did before it could be turned over:
    /// it reads downwards and follows the bottom edge.
    #[default]
    NewestBottom,
    /// The newest line at the top, where the eye already is — what a second
    /// screen glanced at between other work is usually wanted to be.
    NewestTop,
}

/// The three numbers that decide how the pipeline cuts and what it listens
/// to (P12).
///
/// They were constants in the code until 2026-09-21, held back on purpose
/// until the latency/accuracy sweep in P14 could say what they should be.
/// P14 was set aside that day in favour of testing in the house, which
/// inverts the argument rather than removing it: a number that is now found
/// by ear has to be reachable while listening, and an operator who hears a
/// word being cut off has to be able to do something about it before the next
/// cue rather than after the show.
///
/// **Whole numbers, and milliseconds rather than floats.** The file is one an
/// operator may have to read or repair by hand, `550` is easier to be sure
/// about than `0.55`, and it keeps this comparable with `==` — which
/// [`save_config`](crate::commands) relies on to tell a configuration that
/// changed from one that did not.
///
/// Beside the profiles rather than inside one, for the vocabulary's reason
/// rather than the model's: how hard the intercom is to hear — how much air a
/// beltpack puts on the line, how clipped the house's cue calls are — is a
/// property of the system in the building, and it outlives any one
/// production. A patch carried to the next show should not arrive with the
/// detector set for a different room.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "config.ts")]
pub struct Tuning {
    /// How long a pause has to last before it ends a segment.
    ///
    /// The *latency* control, and the one an operator reaches for first: it is
    /// paid in full on every line of the chat, before the ~330 ms of decoding
    /// that follows it. Lower is faster and cuts more sentences in half.
    pub silence_timeout_ms: u64,
    /// The ceiling on one segment, for a speaker who never pauses.
    ///
    /// The worst case an operator waits, and the only bound on it. Lower is
    /// faster and costs accuracy: whisper is trained on 30 s windows and
    /// degrades badly below about two seconds of audio, which is where the
    /// `[BREAK]`s and the invented sentences come from.
    pub max_segment_ms: u64,
    /// How certain the detector has to be that a frame is speech, in whole
    /// percent.
    ///
    /// Lower hears the quiet ones and decodes more of the room; higher decodes
    /// less and loses a command murmured into a beltpack. One number, not two:
    /// the release threshold follows it at a fixed distance, which is what
    /// keeps a word from falling apart mid-vowel — see
    /// [`audio_core::silero::HYSTERESIS`].
    ///
    /// Percent rather than a probability for this struct's reason, and `u8`
    /// because the range is 0–100 and nothing here needs more.
    pub speech_threshold_percent: u8,
    /// Whether a channel's previous sentence goes into the prompt of its next
    /// segment (P25).
    ///
    /// On by default, and a switch rather than a fixture, because it is the
    /// risky half of the context: a word transcribed wrong once is primed to
    /// come out wrong again, and whisper may continue carried text on a quiet
    /// channel. An operator who hears either has to be able to turn it off
    /// during the show. See [`crate::prompt`] for when it lapses on its own.
    ///
    /// `default = "enabled"` so that a file written before this existed gets
    /// the default rather than `false`.
    #[serde(default = "enabled")]
    pub carry_context: bool,
}

impl Default for Tuning {
    /// The constants as they stood when this became a setting, so that a file
    /// written before it existed and a fresh install behave exactly as the
    /// app did.
    fn default() -> Self {
        let segmenter = audio_core::SegmenterConfig::default();
        Self {
            silence_timeout_ms: segmenter.silence_timeout.as_millis() as u64,
            max_segment_ms: segmenter.max_segment.as_millis() as u64,
            speech_threshold_percent: (audio_core::silero::DEFAULT_ONSET * 100.0).round() as u8,
            carry_context: true,
        }
    }
}

impl Tuning {
    /// The narrowest pause that still ends a segment, and the widest.
    ///
    /// Below ~150 ms the pause inside a compound word ends the segment, which
    /// is not a faster app but a chopped-up one. Above two seconds the wait is
    /// longer than anything the ceiling would allow anyway.
    pub const SILENCE_MS: std::ops::RangeInclusive<u64> = 150..=2_000;

    /// The lowest ceiling worth offering, and the highest.
    ///
    /// Two seconds is where whisper's accuracy starts falling off a cliff, so
    /// it is the floor rather than a step on the way to a smaller one.
    /// Fifteen seconds is well past useful and is here only so that a house
    /// that wants whole paragraphs is not stopped by this.
    pub const MAX_SEGMENT_MS: std::ops::RangeInclusive<u64> = 2_000..=15_000;

    /// Silence that ends a segment, in range.
    pub fn silence_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_millis(
            self.silence_timeout_ms
                .clamp(*Self::SILENCE_MS.start(), *Self::SILENCE_MS.end()),
        )
    }

    /// The ceiling, in range.
    pub fn max_segment(&self) -> std::time::Duration {
        std::time::Duration::from_millis(
            self.max_segment_ms
                .clamp(*Self::MAX_SEGMENT_MS.start(), *Self::MAX_SEGMENT_MS.end()),
        )
    }

    /// The detector's onset threshold as a probability.
    pub fn speech_threshold(&self) -> f32 {
        self.speech_threshold_percent.min(100) as f32 / 100.0
    }

    /// Brings every field into range, so that a hand-edited file is corrected
    /// visibly on the Settings page rather than silently at each use.
    fn clamp(&mut self) {
        self.silence_timeout_ms = self.silence_timeout().as_millis() as u64;
        self.max_segment_ms = self.max_segment().as_millis() as u64;
        self.speech_threshold_percent = self.speech_threshold_percent.min(100);
    }
}

/// The whole configuration file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "config.ts", rename = "StoredConfig")]
pub struct Config {
    /// See [`VERSION`]. Present so a later schema change has something to
    /// branch on instead of guessing from the shape of the data.
    pub version: u32,
    /// Position of the profile that was last in use, so an operator who
    /// restarts the app is back where they were.
    ///
    /// Clamped into range on load: a file naming a profile that is not there
    /// should start on the first one, not on none.
    pub active: usize,
    /// The one active vocabulary, session-wide and profile-independent.
    ///
    /// `None` is "never set" and the UI fills in its default; `Some("")` is an
    /// operator who deliberately cleared the box, which turns the prompt off.
    /// Collapsing the two would make the cleared box refill itself on the next
    /// start, which is also how the effect of the vocabulary stops being
    /// measurable (P14).
    pub vocabulary: Option<String>,
    /// Catalogue id of the speech model (P11), e.g. `small-q5_1`, or
    /// [`crate::models::CUSTOM`] for the file [`Config::model_path`] names.
    ///
    /// Beside the profiles rather than inside one, for the same reason the
    /// vocabulary is: which model a machine can run is a property of the
    /// machine — its memory, its GPU, how much latency it can afford — and not
    /// of the production. Carrying a profile to a laptop must not carry a
    /// model choice that laptop cannot serve.
    ///
    /// `None` means the operator never chose, and the default is used.
    /// `#[serde(default)]` throughout this block: a file written before P11 has
    /// none of these fields and must keep loading as "never chose", not fail.
    #[serde(default)]
    pub model: Option<String>,
    /// Catalogue id of the speech-detection model.
    #[serde(default)]
    pub vad_model: Option<String>,
    /// The custom model's `.bin`, from outside the catalogue. The way in for a
    /// machine that will never reach the internet: the operator carries the
    /// file in and says where it is. Used when [`Config::model`] selects it,
    /// and remembered while it does not.
    #[serde(default)]
    pub model_path: Option<String>,
    /// How many whisper sidecars decode in parallel, or `None` for
    /// [`crate::pool::DEFAULT_WORKERS`].
    ///
    /// Beside the profiles like the model, and for the model's reason: how
    /// many decodes a machine can run at once is a property of its GPU and its
    /// memory, not of the production. Carrying a profile to a laptop must not
    /// carry a worker count that laptop cannot serve.
    ///
    /// It is here rather than derived from the channel count because the two
    /// are not the same question. Channels are not capped (see
    /// `session::BUDGETED_STREAMS`); what decides how many of them can be
    /// transcribed without the queue dropping segments is how many decodes run
    /// at once, and only the machine in front of the operator can answer that.
    ///
    /// Clamped into range on use rather than on load, so a file naming a
    /// number this version will not serve keeps saying what it asked for.
    #[serde(default)]
    pub workers: Option<usize>,
    /// **Version 1 only.** The words that must not be missed (P17).
    ///
    /// Read, never written: [`Config::migrate`] turns each term into a rule
    /// that highlights it and asks urgently for attention — what an alert term
    /// did, now written as the trigger and the two actions it always was.
    #[serde(default, skip_serializing)]
    #[ts(skip)]
    pub alerts: Vec<String>,
    /// The LAN remote client (P16): whether it is served, and on which
    /// address.
    ///
    /// Beside the profiles like the vocabulary and the model, and for the
    /// model's reason rather than the vocabulary's: which interface this
    /// machine can be reached on is a property of the machine and of the house
    /// it is standing in, not of the production. Carrying a profile to another
    /// venue must not carry an address that venue does not have.
    ///
    /// `#[serde(default)]` — off, on no address — because a file written
    /// before P16 has no such field, and a version that never offered a
    /// listener must not be read as having asked for one.
    #[serde(default)]
    pub remote: crate::remote::RemoteConfig,
    /// Which end of the chat new lines arrive at, for every window that has
    /// not been turned over (P7).
    ///
    /// Beside the profiles rather than inside one, and unlike the model for
    /// the vocabulary's reason: how a house reads its chat outlives any one
    /// patch, and an operator who has decided which way round it goes has not
    /// decided it per production. Every view the remote serves gets it too —
    /// it is the *default*, and a phone that wants the other one turns itself
    /// over without touching this.
    ///
    /// `#[serde(default)]` — newest at the bottom — because a file written
    /// before this existed has no such field, and what it meant is what the
    /// chat did then.
    #[serde(default)]
    pub chat_sort: SortDirection,
    /// How the pipeline cuts and what it listens to (P12). See [`Tuning`].
    ///
    /// `#[serde(default)]` — the constants the app used to compile in —
    /// because a file written before these were settings describes a session
    /// that ran on exactly those numbers.
    #[serde(default)]
    pub tuning: Tuning,
    /// Whether the session for the active profile starts the moment the app
    /// comes up, without the operator reaching for the Start button.
    ///
    /// Beside the profiles rather than inside one, and for the remote's
    /// reason: whether this machine is meant to be attended when it is
    /// switched on is a property of where it is installed — a booth an
    /// operator sits at all evening against a rack that is powered up before
    /// anyone arrives — not of the production currently patched into it.
    ///
    /// `#[serde(default)]` — off — because a file written before this existed
    /// describes a machine that had to be started by hand, and that is what
    /// its absence must keep meaning.
    #[serde(default)]
    pub auto_start: bool,
    /// Whether the first-run setup guide has been walked, or left.
    ///
    /// Not "has a model": that question is asked of the disk on every start
    /// and answered by the model manager. This is the narrower one the guide
    /// itself needs — *has this operator been shown the way through* — and the
    /// two are different the moment somebody deletes a model in Settings, when
    /// the right answer is the section they are already standing in rather
    /// than a welcome screen.
    ///
    /// `#[serde(default)]` — false — because a file written before the guide
    /// existed cannot say. An installation that is already set up is not made
    /// to sit through it: the app settles the flag at startup when it finds a
    /// working model, and only shows the guide to a machine that has neither.
    #[serde(default)]
    pub setup_done: bool,
    pub profiles: Vec<Profile>,
}

impl Config {
    /// The model manager's view of this file.
    pub fn selection(&self) -> crate::models::Selection {
        crate::models::Selection {
            speech: self.model.clone(),
            vad: self.vad_model.clone(),
            path: self.model_path.clone(),
        }
    }
}

impl Default for Config {
    /// Empty, not a specimen setup: the channel map that makes sense depends
    /// on an interface this code has not seen. The UI creates the first
    /// profile once it knows what is plugged in.
    fn default() -> Self {
        Self {
            version: VERSION,
            active: 0,
            vocabulary: None,
            model: None,
            vad_model: None,
            model_path: None,
            workers: None,
            alerts: Vec::new(),
            remote: crate::remote::RemoteConfig::default(),
            chat_sort: SortDirection::default(),
            tuning: Tuning::default(),
            auto_start: false,
            setup_done: false,
            profiles: Vec::new(),
        }
    }
}

impl Config {
    /// This configuration in the form the file holds it.
    ///
    /// Every configuration that reaches the disk goes through here, whichever
    /// end it came from — which is what makes "is this the same as what is
    /// already stored" a question with an answer. A view that sends a
    /// configuration the file would have tidied up would otherwise differ from
    /// the stored one forever, and every save would look like a change.
    pub fn sanitised(mut self) -> Self {
        self.sanitise();
        self
    }

    /// Brings a file that has been edited, truncated or written by another
    /// version into a state the rest of the app can rely on.
    fn sanitise(&mut self) {
        for profile in &mut self.profiles {
            let channels = profile.channels.len();
            let groups = profile.groups.len();
            for group in &mut profile.groups {
                group.members.retain(|&m| m < channels);
            }
            profile.outputs.shoutbox.clamp();
            profile.outputs.listen.keep_minutes = profile.outputs.listen.keep_minutes.min(120);
            // A rule pointing at a channel or a group that is not there any
            // more is the one failure mode a rule has that an alert term did
            // not: it looks like a narrowed rule and behaves like a broader
            // one. Dropped on the same terms group membership is — the rule
            // loses the channel, not the operator their rule — except where
            // dropping would *widen* it, which is what `enabled` catches
            // below.
            for rule in &mut profile.rules {
                let located = !rule.channels.is_empty();
                let addressed = !rule.views.is_empty();
                rule.channels.retain(|&c| c < channels);
                rule.views.retain(|&v| v < groups);
                // Everything it named is gone. Left in the file, so the
                // operator can see what it said and re-point it, but switched
                // off: a rule that was "Feuer on the stage manager's channel"
                // must not quietly become "Feuer everywhere" because the
                // channel was deleted, and one addressed to one window must
                // not become one that floods them all because the group was.
                if (located && rule.channels.is_empty()) || (addressed && rule.views.is_empty()) {
                    rule.enabled = false;
                }
            }
        }

        if self.active >= self.profiles.len() {
            self.active = 0;
        }

        self.tuning.clamp();

        // A key with a space on the end is the key without it, and the two
        // have to be one string here: `save_config` decides whether a save
        // changed anything by comparing tidied configurations, and a field
        // only one side trims is a field that makes every save look like a
        // change. Trimmed on the frontend's side too — see `toStoredConfig`.
        if self.remote.key.trim().len() != self.remote.key.len() {
            self.remote.key = self.remote.key.trim().to_string();
        }

        self.version = VERSION;
    }

    /// This configuration as a browser the LAN remote serves is shown it.
    ///
    /// Three things are the machine's alone, and the backend — not the page —
    /// is what keeps them there: the remote's own settings (the key above
    /// all), the custom model's path on this disk, and every webhook's
    /// destination, headers and body (P19), which may carry a token. A
    /// webhook is kept as [`Webhook::redacted`], so the remote still sees
    /// *that* a rule calls out and can switch it off.
    pub fn for_remote(&self, secret: &[u8; 32]) -> Config {
        let mut shown = self.clone();
        shown.remote.key = String::new();
        shown.model_path = None;
        for rule in shown.profiles.iter_mut().flat_map(|p| p.rules.iter_mut()) {
            rule.webhook = rule.webhook.as_ref().map(|hook| hook.redacted(secret));
        }
        shown
    }

    /// A configuration a remote browser saved, with what only the machine may
    /// write put back from `stored`.
    ///
    /// Whatever the remote sent for those is ignored: its remote settings and
    /// model path are replaced with the stored ones, and a webhook survives
    /// only as a name [`Config::for_remote`] handed out — so a remote can keep
    /// a request or drop it, and cannot write one. A name that matches nothing
    /// is a remote a save behind the machine, which changed the request in the
    /// meantime; the rule in the same place under the same name keeps its
    /// current one rather than losing it to a stale view.
    pub fn from_remote(mut self, stored: &Config, secret: &[u8; 32]) -> Config {
        self.remote = stored.remote.clone();
        self.model_path = stored.model_path.clone();

        // The listen output is the host's headphones (P22): a remote hears a
        // replay in its own browser, so it has no business routing the host's.
        // The shoutbox (P26) is the remote's to set up in full — patch, curve
        // and calibration — because the person tuning it is at the position,
        // not at the laptop; `sanitise` clamps what it sends like anything
        // else. A profile the machine does not have yet gets the default
        // listen output rather than the remote's.
        for (p, profile) in self.profiles.iter_mut().enumerate() {
            profile.outputs.listen = stored
                .profiles
                .get(p)
                .map(|same| same.outputs.listen.clone())
                .unwrap_or_default();
        }

        let known = stored.webhooks_by_fingerprint(secret);
        for (p, profile) in self.profiles.iter_mut().enumerate() {
            for (r, rule) in profile.rules.iter_mut().enumerate() {
                let Some(sent) = rule.webhook.take() else {
                    continue;
                };
                rule.webhook = known.get(&sent.url).cloned().or_else(|| {
                    stored
                        .profiles
                        .get(p)?
                        .rules
                        .get(r)
                        .filter(|same| same.name == rule.name)?
                        .webhook
                        .clone()
                });
            }
        }
        self
    }

    /// Every webhook in the file, by the name the remote knows it as.
    pub fn webhooks_by_fingerprint(&self, secret: &[u8; 32]) -> HashMap<String, Webhook> {
        self.profiles
            .iter()
            .flat_map(|p| p.rules.iter())
            .filter_map(|rule| rule.webhook.as_ref())
            .map(|hook| (hook.fingerprint(secret), hook.clone()))
            .collect()
    }

    /// Turns a version 1 file into a version 2 one: the alert list and the two
    /// flash flags become rules (P19).
    ///
    /// Nothing is lost and nothing has to be re-entered. Each alert term
    /// becomes a rule that highlights it and asks urgently for attention,
    /// which is what an alert term did; each group with its flash flag on
    /// becomes a rule triggered by that group's *channels* and addressed to
    /// that group's *window*, which is exactly what the flag did; the
    /// all-channels view's flag becomes a rule with no trigger at all.
    ///
    /// The terms were global and the rules are per profile, so each profile
    /// gets its own copy of them — the alternative is an operator whose alert
    /// terms work in one production and are missing in the next, which is the
    /// exact failure the global list existed to prevent.
    ///
    /// Run on the way *in*, so it costs nothing on a file that has already
    /// been migrated: a version 2 file has no legacy fields to read.
    fn migrate(&mut self) {
        if self.version < 3 {
            for profile in &mut self.profiles {
                profile.outputs.shoutbox.lift_to_spl();
            }
        }

        let terms = std::mem::take(&mut self.alerts);

        for profile in &mut self.profiles {
            let mut rules = Vec::new();

            for term in &terms {
                rules.push(StoredRule {
                    name: term.clone(),
                    enabled: true,
                    words: vec![term.clone()],
                    channels: Vec::new(),
                    views: Vec::new(),
                    actions: vec![rules::Action::Highlight, rules::Action::UrgentAttention],
                    webhook: None,
                    min_interval_ms: 0,
                });
            }

            for (position, group) in profile.groups.iter().enumerate() {
                if !group.flash {
                    continue;
                }
                rules.push(StoredRule {
                    name: group.name.clone(),
                    enabled: true,
                    words: Vec::new(),
                    // Both halves, because the flag was both: it fired on what
                    // the group could hear and it flooded the window of that
                    // group. Written as one list each, it is now sayable — and
                    // separable, which it never was.
                    channels: group.members.clone(),
                    views: vec![position],
                    actions: vec![rules::Action::Flash],
                    webhook: None,
                    min_interval_ms: 0,
                });
            }

            if profile.all_channels_flash {
                rules.push(StoredRule {
                    // Named after what it does, because the flag it comes from
                    // had no name: it was a checkbox on a view.
                    name: "Every message".to_string(),
                    enabled: true,
                    words: Vec::new(),
                    channels: Vec::new(),
                    views: Vec::new(),
                    actions: vec![rules::Action::Flash],
                    webhook: None,
                    min_interval_ms: 0,
                });
            }

            for group in &mut profile.groups {
                group.flash = false;
            }
            profile.all_channels_flash = false;

            // Ahead of whatever the file already held, which for a version 1
            // file is nothing. Appending rather than replacing is what makes
            // running this twice harmless.
            rules.extend(std::mem::take(&mut profile.rules));
            profile.rules = rules;
        }
    }
}

/// Reads the configuration, or returns the default one.
///
/// Nothing here is allowed to stop the app from starting. A missing file is
/// the normal first run. A file that cannot be parsed — hand-edited, truncated
/// by a full disk, written by a future version — is moved aside to
/// `config.bad.json` and reported, and the app comes up empty rather than
/// refusing to open or, worse, quietly overwriting whatever was in there
/// before anyone could look at it.
pub fn load(path: &Path) -> Config {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            tracing::info!(path = %path.display(), "no configuration file yet");
            return Config::default();
        }
        Err(err) => {
            tracing::error!(path = %path.display(), error = %err, "cannot read the configuration");
            return Config::default();
        }
    };

    match serde_json::from_str::<Config>(&text) {
        Ok(mut config) if config.version <= VERSION => {
            // Before the tidying, because it is what fills in the rules the
            // tidying then prunes: a version 1 file's groups become group
            // triggers, and a hand-truncated channel list has to cost one of
            // those its channel like any other.
            if config.version < VERSION {
                tracing::info!(
                    from = config.version,
                    to = VERSION,
                    "migrating the configuration"
                );
                config.migrate();
            }
            config.sanitise();
            tracing::info!(
                profiles = config.profiles.len(),
                active = config.active,
                "configuration loaded"
            );
            config
        }
        Ok(config) => {
            // Forward compatibility is not something this can fake: a newer
            // file may hold fields that carry meaning, and saving over it
            // would drop them. Keep it, start clean, say so.
            tracing::error!(
                version = config.version,
                supported = VERSION,
                "the configuration was written by a newer version"
            );
            quarantine(path);
            Config::default()
        }
        Err(err) => {
            tracing::error!(error = %err, "the configuration file is not readable as one");
            quarantine(path);
            Config::default()
        }
    }
}

/// Moves an unusable file out of the way, so the next save does not destroy
/// the only copy of a setup that may still be recoverable by hand.
fn quarantine(path: &Path) {
    let aside = path.with_extension("bad.json");
    match fs::rename(path, &aside) {
        Ok(()) => tracing::warn!(path = %aside.display(), "kept the unusable configuration"),
        Err(err) => tracing::error!(error = %err, "could not keep the unusable configuration"),
    }
}

/// Writes the configuration, atomically.
///
/// Through a temporary file and a rename, because the alternative is a
/// truncated `config.json`: the app saves on every edit on the Settings page,
/// so a crash or a power loss lands in the middle of a write far more often
/// than the rarity of either suggests. A rename within one directory either
/// happens or does not, and the cost of getting this wrong is the setup the
/// whole package exists to preserve.
pub fn save(path: &Path, config: &Config) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)
            .with_context(|| format!("creating the configuration directory {}", dir.display()))?;
    }

    // Pretty-printed: this is a file an operator may open to see what their
    // setup actually says, or send to someone else when it is wrong.
    let text = serde_json::to_string_pretty(config).context("serialising the configuration")?;

    let temp = temp_path(path);
    fs::write(&temp, text).with_context(|| format!("writing {}", temp.display()))?;
    fs::rename(&temp, path).with_context(|| format!("replacing {}", path.display()))?;

    Ok(())
}

/// The temporary file a save goes through. Next to the target, because a
/// rename across filesystems is not atomic — and `/tmp` is regularly a
/// different filesystem.
fn temp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A directory of this test's own. No `tempfile` dependency for four
    /// tests, and the counter is what keeps two of them in the same process
    /// from sharing a path.
    fn temp_dir() -> PathBuf {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "fluestr-config-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        // Emptied first. The name is unique within one process and no further:
        // a pid is reused, and which test gets which number depends on the
        // order they happen to start in, so a run can be handed the directory
        // a *different* test filled on some earlier run. What that looks like
        // is `a_first_start_has_no_file_and_no_complaint` failing now and then
        // on a machine that has run these tests a lot, and passing on its own
        // every time it is investigated.
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("creating the test directory");
        dir
    }

    fn a_config() -> Config {
        Config {
            version: VERSION,
            active: 1,
            vocabulary: Some("Standby, Cue, Go.".into()),
            model: Some("small-q5_1".into()),
            vad_model: Some("silero-v5.1.2".into()),
            model_path: None,
            workers: Some(4),
            // Version 1 only, and never written: a fixture that set them
            // would not survive its own round trip.
            alerts: Vec::new(),
            remote: crate::remote::RemoteConfig {
                enabled: true,
                address: "192.168.1.20".into(),
                port: 8787,
                key: "k7p4m2rq".into(),
            },
            chat_sort: SortDirection::NewestTop,
            tuning: Tuning {
                silence_timeout_ms: 350,
                max_segment_ms: 4_000,
                speech_threshold_percent: 60,
                carry_context: false,
            },
            auto_start: true,
            setup_done: true,
            profiles: vec![
                Profile {
                    name: "Probebühne".into(),
                    device: "Scarlett 2i2".into(),
                    device_id: "AppleUSBAudioEngine:Focusrite:Scarlett 2i2:1".into(),
                    language: "de".into(),
                    channels: vec![StoredChannel {
                        channel: 0,
                        label: "Regie".into(),
                        language: "de".into(),
                        color: "#6ea8fe".into(),
                        gain_db: 0,
                        muted: false,
                        description: String::new(),
                    }],
                    groups: vec![],
                    all_channels_flash: false,
                    rules: vec![],
                    outputs: Outputs::default(),
                },
                Profile {
                    name: "Grosses Haus".into(),
                    device: "MADIface".into(),
                    device_id: "AppleUSBAudioEngine:RME:MADIface:2".into(),
                    language: "en".into(),
                    channels: vec![
                        StoredChannel {
                            channel: 0,
                            label: "Regie".into(),
                            language: "de".into(),
                            color: "#6ea8fe".into(),
                            gain_db: 0,
                            muted: false,
                            description: String::new(),
                        },
                        StoredChannel {
                            channel: 3,
                            label: "Licht".into(),
                            language: "en".into(),
                            color: "#5fd38d".into(),
                            // The trim this profile needs on the follow-spot's
                            // matrix output — and the field a file written
                            // before it existed does not have at all.
                            gain_db: 9,
                            // Muted for this production: the position is
                            // patched but nobody sits on it, and it would
                            // otherwise fill the chat with the room it is in.
                            muted: true,
                            description: "Licht. Verfolger 2, Preset 12, Blackout.".into(),
                        },
                    ],
                    groups: vec![StoredGroup {
                        name: "Bühne".into(),
                        members: vec![1],
                        flash: false,
                        on_top: true,
                    }],
                    all_channels_flash: false,
                    rules: vec![
                        StoredRule {
                            name: "Feuer".into(),
                            enabled: true,
                            words: vec!["Feuer".into()],
                            channels: vec![],
                            views: vec![],
                            actions: vec![rules::Action::Highlight, rules::Action::UrgentAttention],
                            webhook: None,
                            min_interval_ms: 0,
                        },
                        StoredRule {
                            name: "Go bei der Inspizienz".into(),
                            enabled: true,
                            words: vec!["Go".into()],
                            channels: vec![1],
                            views: vec![0],
                            actions: vec![rules::Action::Flash],
                            webhook: None,
                            min_interval_ms: 1_000,
                        },
                    ],
                    // Patched, so the round trip covers every field of it.
                    outputs: Outputs {
                        listen: Listen {
                            output: OutputRoute {
                                device: "MADIface".into(),
                                device_id: "AppleUSBAudioEngine:RME:MADIface:2".into(),
                                channels: vec![6, 7],
                            },
                            keep_minutes: 15,
                        },
                        shoutbox: Shoutbox {
                            enabled: true,
                            program: InputRoute {
                                device: "MADIface".into(),
                                device_id: "AppleUSBAudioEngine:RME:MADIface:2".into(),
                                channel: 3,
                            },
                            reference: InputRoute {
                                device: "MADIface".into(),
                                device_id: "AppleUSBAudioEngine:RME:MADIface:2".into(),
                                channel: 11,
                            },
                            output: OutputRoute {
                                device: "MADIface".into(),
                                device_id: "AppleUSBAudioEngine:RME:MADIface:2".into(),
                                channels: vec![4],
                            },
                            calibration_db: Some(Tenths(1183)),
                            quiet_db: 70,
                            trim_db: -3,
                            ..Shoutbox::default()
                        },
                    },
                },
            ],
        }
    }

    #[test]
    fn a_saved_configuration_comes_back_unchanged() {
        // The whole point of the package: what the operator built is what the
        // next start finds — colours, languages, group membership and which
        // profile was in use included.
        let path = temp_dir().join(FILE_NAME);
        let config = a_config();

        save(&path, &config).expect("saving");

        assert_eq!(load(&path), config);
    }

    #[test]
    fn a_first_start_has_no_file_and_no_complaint() {
        let path = temp_dir().join(FILE_NAME);

        assert_eq!(load(&path), Config::default());
        assert!(!path.exists(), "loading must not create anything");
    }

    #[test]
    fn an_unreadable_file_is_kept_rather_than_overwritten() {
        // It may still be recoverable by hand, and it is the only copy of a
        // setup. Starting empty is recoverable; starting by destroying it is
        // not.
        let dir = temp_dir();
        let path = dir.join(FILE_NAME);
        fs::write(&path, "{ this is not json").expect("writing the broken file");

        assert_eq!(load(&path), Config::default());
        assert_eq!(
            fs::read_to_string(dir.join("config.bad.json")).expect("the file kept aside"),
            "{ this is not json"
        );
    }

    #[test]
    fn a_file_from_a_newer_version_is_not_reinterpreted() {
        // It may hold fields this version knows nothing about, and saving over
        // it would drop them.
        let dir = temp_dir();
        let path = dir.join(FILE_NAME);
        let mut config = a_config();
        config.version = VERSION + 1;
        save(&path, &config).expect("saving");

        assert_eq!(load(&path), Config::default());
        assert!(dir.join("config.bad.json").exists());
    }

    #[test]
    fn a_file_from_before_the_alert_list_still_loads() {
        // Every field P17 added is optional on the way in, so a setup built
        // with an earlier version comes back whole — and, having asked for
        // neither alert terms nor flashing, it comes back with no rules.
        let path = temp_dir().join(FILE_NAME);
        fs::write(
            &path,
            // `r##`, because the colour below ends in `"#` and would otherwise
            // close an `r#` string in the middle of the fixture.
            r##"{
              "version": 1,
              "active": 0,
              "vocabulary": null,
              "profiles": [
                {
                  "name": "Probebühne",
                  "device": "Scarlett 2i2",
                  "channels": [
                    { "channel": 0, "label": "Regie", "language": "de", "color": "#6ea8fe" }
                  ],
                  "groups": [{ "name": "Bühne", "members": [0] }]
                }
              ]
            }"##,
        )
        .expect("writing the older file");

        let config = load(&path);

        assert!(config.profiles[0].rules.is_empty());
        // And it decodes with the default worker count rather than none.
        assert_eq!(config.workers, None);
        assert_eq!(config.profiles[0].groups[0].members, vec![0]);
        // And its window opens like any other one: a setup that suddenly put
        // windows in front of everything is not what a missing field meant.
        assert!(!config.profiles[0].groups[0].on_top);
        // And its chat reads the way it read then: a file with nothing to say
        // about the direction must not come back turned over.
        assert_eq!(config.chat_sort, SortDirection::NewestBottom);
        // And its channels have nothing to describe them to the model.
        assert_eq!(config.profiles[0].channels[0].description, "");
    }

    /// The acceptance criterion P19 is signed off against: a version 1 file
    /// comes back as the equivalent rules, without the operator re-entering
    /// anything.
    #[test]
    fn a_version_one_file_comes_back_as_rules() {
        let path = temp_dir().join(FILE_NAME);
        fs::write(
            &path,
            r##"{
              "version": 1,
              "active": 0,
              "vocabulary": null,
              "alerts": ["Feuer", "Arzt"],
              "profiles": [
                {
                  "name": "Grosses Haus",
                  "device": "MADIface",
                  "channels": [
                    { "channel": 0, "label": "Regie", "language": "de", "color": "#6ea8fe" },
                    { "channel": 3, "label": "Licht", "language": "de", "color": "#5fd38d" }
                  ],
                  "groups": [
                    { "name": "Bühne", "members": [1], "flash": true },
                    { "name": "Licht", "members": [1], "flash": false }
                  ],
                  "allChannelsFlash": true
                }
              ]
            }"##,
        )
        .expect("writing the version 1 file");

        let config = load(&path);
        assert_eq!(config.version, VERSION);
        let rules = &config.profiles[0].rules;

        // Two terms, the one flashing group, and the all-channels flag.
        assert_eq!(rules.len(), 4);

        assert_eq!(rules[0].words, ["Feuer"]);
        assert!(rules[0].channels.is_empty() && rules[0].views.is_empty());
        assert_eq!(
            rules[0].actions,
            [rules::Action::Highlight, rules::Action::UrgentAttention],
            "an alert term highlighted and asked urgently — both, and nothing else"
        );
        assert_eq!(rules[1].words, ["Arzt"]);

        // The group that flashed, as the two halves it always was: its
        // channels are when it fires, its window is who is told.
        assert_eq!(rules[2].views, [0]);
        assert_eq!(rules[2].channels, [1]);
        assert!(rules[2].words.is_empty());
        assert_eq!(rules[2].actions, [rules::Action::Flash]);

        // The all-channels view's flag: no trigger at all.
        assert!(rules[3].words.is_empty());
        assert!(rules[3].channels.is_empty() && rules[3].views.is_empty());
        assert_eq!(rules[3].actions, [rules::Action::Flash]);

        // And the flags they came from are spent, so re-saving cannot produce
        // the same rules a second time.
        assert!(config.alerts.is_empty());
        assert!(!config.profiles[0].groups[0].flash);
        assert!(!config.profiles[0].all_channels_flash);
    }

    /// P19's other acceptance criterion: deleting a channel leaves no rule
    /// pointing at nothing — and, where that was the whole of the rule's
    /// trigger, no rule that has quietly become louder than it was written.
    #[test]
    fn a_rule_pointing_past_the_channels_loses_the_channel_and_not_its_narrowing() {
        let path = temp_dir().join(FILE_NAME);
        let mut config = a_config();
        config.profiles[1].rules[1].channels = vec![1, 9];
        config.profiles[1].rules.push(StoredRule {
            name: "nur auf Kanal 9".into(),
            enabled: true,
            words: vec!["Go".into()],
            channels: vec![9],
            views: vec![],
            actions: vec![rules::Action::Flash],
            webhook: None,
            min_interval_ms: 0,
        });
        save(&path, &config).expect("saving");

        let loaded = load(&path);
        let rules = &loaded.profiles[1].rules;
        assert_eq!(rules[1].channels, [1], "the channel that is there survives");
        assert!(rules[1].enabled);

        assert!(
            rules[2].channels.is_empty() && !rules[2].enabled,
            "a rule whose every channel is gone is kept and switched off, never widened"
        );
    }

    #[test]
    fn membership_pointing_past_the_channels_costs_the_member_not_the_setup() {
        let path = temp_dir().join(FILE_NAME);
        let mut config = a_config();
        config.profiles[1].groups[0].members = vec![0, 9];
        save(&path, &config).expect("saving");

        let loaded = load(&path);
        assert_eq!(loaded.profiles[1].groups[0].members, vec![0]);
        assert_eq!(loaded.profiles[1].channels.len(), 2, "nothing else is lost");
    }

    #[test]
    fn an_active_profile_that_is_not_there_falls_back_to_the_first() {
        let path = temp_dir().join(FILE_NAME);
        let mut config = a_config();
        config.active = 7;
        save(&path, &config).expect("saving");

        assert_eq!(load(&path).active, 0);
    }

    #[test]
    fn saving_twice_leaves_one_file_and_no_leftovers() {
        // The temporary file the atomic write goes through must not survive
        // it — a `config.json.tmp` next to the real one looks like a crash to
        // whoever finds it.
        let dir = temp_dir();
        let path = dir.join(FILE_NAME);

        save(&path, &Config::default()).expect("saving");
        save(&path, &a_config()).expect("saving again");

        let left: Vec<String> = fs::read_dir(&dir)
            .expect("reading the directory")
            .map(|e| e.expect("an entry").file_name().to_string_lossy().into())
            .collect();
        assert_eq!(left, vec![FILE_NAME.to_string()]);
        assert_eq!(load(&path), a_config());
    }

    #[test]
    fn the_directory_is_created_on_the_first_save() {
        // On a fresh machine nothing has created the app's configuration
        // directory yet, and the first save is what discovers that.
        let path = temp_dir().join("nested").join(FILE_NAME);

        save(&path, &a_config()).expect("saving into a directory that does not exist");
        assert_eq!(load(&path), a_config());
    }

    #[test]
    fn a_file_written_before_the_model_manager_still_loads() {
        // P11 added three fields. An operator upgrading has a file without
        // them, and losing their whole setup to a schema addition — profiles,
        // groups, colours — would be the worst possible trade for a picker.
        let dir = temp_dir();
        let path = dir.join(FILE_NAME);
        fs::write(
            &path,
            r##"{"version":1,"active":0,"vocabulary":"Standby, Cue, Go.",
                "profiles":[{"name":"Probebühne","device":"Scarlett 2i2",
                  "channels":[{"channel":0,"label":"Regie","language":"de","color":"#6ea8fe"}],
                  "groups":[]}]}"##,
        )
        .expect("writing the old file");

        let loaded = load(&path);
        assert_eq!(loaded.profiles.len(), 1, "the setup survived");
        assert_eq!(loaded.profiles[0].channels[0].label, "Regie");
        // "never chose", which resolves to the default rather than to nothing.
        assert_eq!(loaded.model, None);
        assert_eq!(loaded.vad_model, None);
        assert_eq!(loaded.model_path, None);
        // Same for the stable device id, which the file also predates. Empty
        // rather than guessed: the name is still there and still resolves, and
        // the id is filled in the first time that device is seen connected —
        // an existing setup earns one by being used, not by being re-entered.
        assert_eq!(loaded.profiles[0].device, "Scarlett 2i2");
        assert_eq!(loaded.profiles[0].device_id, "");
        // Same for the profile's default language, which the file predates: a
        // profile without one has to keep loading, on the default rather than
        // on an empty string that would make a new channel languageless.
        assert_eq!(loaded.profiles[0].language, "de");
        assert!(
            !dir.join("config.bad.json").exists(),
            "nothing was quarantined"
        );
    }

    #[test]
    fn a_file_written_before_the_thresholds_were_settings_keeps_the_constants() {
        // The three numbers were compiled in until P12 exposed them. A file
        // from before that describes a session that ran on exactly those
        // values, so reading it must produce them and not a new opinion.
        let path = temp_dir().join(FILE_NAME);
        fs::write(
            &path,
            r##"{"version":2,"active":0,"vocabulary":null,"profiles":[]}"##,
        )
        .expect("writing the old file");

        let loaded = load(&path);
        assert_eq!(loaded.tuning, Tuning::default());
        let segmenter = audio_core::SegmenterConfig::default();
        assert_eq!(loaded.tuning.silence_timeout(), segmenter.silence_timeout);
        assert_eq!(loaded.tuning.max_segment(), segmenter.max_segment);
        assert_eq!(
            loaded.tuning.speech_threshold(),
            audio_core::silero::DEFAULT_ONSET
        );
    }

    #[test]
    fn a_hand_edited_threshold_is_corrected_where_it_can_be_seen() {
        // Clamping only at each use would leave the Settings page showing a
        // number the pipeline is not running on. Correcting it on the way
        // through the file puts the page and the audio back in agreement, and
        // the operator can see that their 10 ms was not taken.
        let path = temp_dir().join(FILE_NAME);
        fs::write(
            &path,
            r##"{"version":2,"active":0,"vocabulary":null,"profiles":[],
                "tuning":{"silenceTimeoutMs":10,"maxSegmentMs":600000,
                          "speechThresholdPercent":250}}"##,
        )
        .expect("writing the file");

        let loaded = load(&path);
        assert_eq!(
            loaded.tuning.silence_timeout_ms,
            *Tuning::SILENCE_MS.start()
        );
        assert_eq!(loaded.tuning.max_segment_ms, *Tuning::MAX_SEGMENT_MS.end());
        assert_eq!(loaded.tuning.speech_threshold_percent, 100);
        assert_eq!(loaded.tuning.speech_threshold(), 1.0);
        // Written before the switch existed: the context is carried, which is
        // the default, rather than switched off by a field that is missing.
        assert!(loaded.tuning.carry_context);
    }

    #[test]
    fn a_cleared_vocabulary_stays_cleared() {
        // `None` is "never set" and the UI fills in its default; `Some("")` is
        // an operator who turned the prompt off. Collapsing the two would
        // refill the box they emptied on the next start.
        let path = temp_dir().join(FILE_NAME);
        let mut config = a_config();
        config.vocabulary = Some(String::new());
        save(&path, &config).expect("saving");

        assert_eq!(load(&path).vocabulary, Some(String::new()));
    }

    /// The fields the frontend has to write, exactly.
    ///
    /// The configuration is announced to every view after it is saved, and a
    /// view recognises the echo of its own save by comparing what came back
    /// with what it sent (P16). That comparison is order-independent, so what
    /// it cannot survive is a field one side writes and the other does not:
    /// every save would then read as somebody else's change, the Settings page
    /// would adopt its own keystroke, and the field being typed in would be
    /// rebuilt under the cursor.
    ///
    /// Pinned here rather than left to be discovered, because adding a field
    /// to this struct without adding it to `toStoredConfig` in `profiles.ts`
    /// is a one-line change with that as its symptom.
    #[test]
    fn the_file_holds_exactly_what_the_frontend_writes() {
        let json = serde_json::to_value(a_config()).expect("serialising");

        assert_eq!(
            keys(&json),
            [
                "active",
                "autoStart",
                "chatSort",
                "model",
                "modelPath",
                "profiles",
                "remote",
                "setupDone",
                "tuning",
                "vadModel",
                "version",
                "vocabulary",
                "workers",
            ]
        );
        assert_eq!(
            keys(&json["tuning"]),
            [
                "carryContext",
                "maxSegmentMs",
                "silenceTimeoutMs",
                "speechThresholdPercent"
            ]
        );
        assert_eq!(
            keys(&json["profiles"][0]),
            ["channels", "device", "deviceId", "groups", "language", "name", "outputs", "rules"]
        );
        assert_eq!(
            keys(&json["profiles"][1]["outputs"]),
            ["listen", "shoutbox"]
        );
        assert_eq!(
            keys(&json["profiles"][1]["outputs"]["listen"]),
            ["keepMinutes", "output"]
        );
        assert_eq!(
            keys(&json["profiles"][1]["outputs"]["shoutbox"]["output"]),
            ["channels", "device", "deviceId"]
        );
        assert_eq!(
            keys(&json["profiles"][1]["outputs"]["shoutbox"]["program"]),
            ["channel", "device", "deviceId"]
        );
        assert_eq!(
            keys(&json["profiles"][0]["channels"][0]),
            [
                "channel",
                "color",
                "description",
                "gainDb",
                "label",
                "language",
                "muted"
            ]
        );
        // The second profile, because it is the one with groups in it.
        assert_eq!(
            keys(&json["profiles"][1]["groups"][0]),
            ["members", "name", "onTop"]
        );
        assert_eq!(
            keys(&json["profiles"][1]["rules"][0]),
            [
                "actions",
                "channels",
                "enabled",
                "minIntervalMs",
                "name",
                "views",
                "webhook",
                "words"
            ]
        );
        assert_eq!(keys(&json["remote"]), ["address", "enabled", "key", "port"]);
    }

    /// An object's field names, sorted — the same order the frontend's
    /// comparison puts them in.
    fn keys(value: &serde_json::Value) -> Vec<&str> {
        let mut names: Vec<&str> = value
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        names.sort_unstable();
        names
    }

    /// Tidying has to be a fixpoint.
    ///
    /// `save_config` compares what it was sent with what the file holds and
    /// does nothing when they agree (P16). Both sides of that comparison have
    /// been tidied, so a tidy that changed something every time it ran would
    /// make every save look like a change — which is the save storm the
    /// comparison exists to prevent.
    #[test]
    fn tidying_a_tidy_configuration_changes_nothing() {
        let once = a_config().sanitised();
        assert_eq!(once.clone().sanitised(), once);
    }

    /// A configuration that only differs by what the tidy would remove is not
    /// a change: a view that sends a group member the channel list no longer
    /// has must not have that written back at it forever.
    #[test]
    fn a_configuration_that_only_needs_tidying_is_not_a_change() {
        let stored = a_config().sanitised();

        let mut sent = stored.clone();
        // A member position the channel list does not reach. The tidy drops
        // it, here and on the way in, so the two agree.
        sent.profiles[1].groups[0].members.push(99);

        assert_eq!(sent.sanitised(), stored);
    }

    fn with_a_webhook() -> Config {
        let mut config = a_config().sanitised();
        config.model_path = Some("/Volumes/Stick/ggml-medium.bin".into());
        config.profiles[1].rules[1]
            .actions
            .push(rules::Action::Webhook);
        config.profiles[1].rules[1].webhook = Some(Webhook {
            url: "http://companion:8000/api/location/1/0/2/press".into(),
            method: crate::webhook::Method::Post,
            headers: vec![crate::webhook::Header {
                name: "Authorization".into(),
                value: "Bearer house-token".into(),
            }],
            body: "{{text}}".into(),
        });
        config
    }

    /// The machine's half never reaches a remote browser: not the key, not a
    /// path on this disk, not where a webhook goes or what it carries.
    #[test]
    fn a_remote_is_not_shown_what_only_the_machine_may_write() {
        let secret = [3u8; 32];
        let shown = serde_json::to_string(&with_a_webhook().for_remote(&secret)).unwrap();

        for secret_part in [
            "k7p4m2rq",
            "ggml-medium",
            "companion",
            "house-token",
            "{{text}}",
        ] {
            assert!(
                !shown.contains(secret_part),
                "{secret_part} reached the remote"
            );
        }
        // But it still sees that the rule calls out, which is what lets it
        // switch the rule off.
        assert!(shown.contains(crate::webhook::REDACTED));
    }

    /// A remote that sends back what it was shown changes nothing — the
    /// requests come back from their names.
    #[test]
    fn what_a_remote_was_shown_saves_as_what_is_stored() {
        let secret = [3u8; 32];
        let stored = with_a_webhook();
        let echoed = stored.for_remote(&secret).from_remote(&stored, &secret);
        assert_eq!(echoed.sanitised(), stored);
    }

    /// A remote cannot write a destination, change its own listener or point
    /// the model at another file, whatever it sends.
    #[test]
    fn a_remote_cannot_write_what_only_the_machine_may() {
        let secret = [3u8; 32];
        let stored = with_a_webhook();

        let mut sent = stored.for_remote(&secret);
        sent.remote.key = String::new();
        sent.remote.address = "0.0.0.0".into();
        sent.model_path = Some("/etc/passwd".into());
        // A new webhook on a rule that had none, and one pointed elsewhere.
        sent.profiles[1].rules[0]
            .actions
            .push(rules::Action::Webhook);
        sent.profiles[1].rules[0].webhook = Some(Webhook {
            url: "https://attacker.example/collect".into(),
            method: crate::webhook::Method::Post,
            headers: vec![],
            body: "{{text}}".into(),
        });

        let saved = sent.from_remote(&stored, &secret);
        assert_eq!(saved.remote, stored.remote);
        assert_eq!(saved.model_path, stored.model_path);
        assert_eq!(saved.profiles[1].rules[0].webhook, None);
        assert_eq!(
            saved.profiles[1].rules[1].webhook,
            stored.profiles[1].rules[1].webhook
        );
    }

    #[test]
    fn a_calibration_keeps_a_tenth_goes_negative_and_reads_old_files() {
        let read = |json: &str| {
            let mut shoutbox: Shoutbox = serde_json::from_str(json).expect("parsing");
            shoutbox.clamp();
            shoutbox.calibration_db
        };
        assert_eq!(read(r#"{"calibrationDb": 118}"#), Some(Tenths(1180)));
        assert_eq!(read(r#"{"calibrationDb": 117.46}"#), Some(Tenths(1175)));
        assert_eq!(read(r#"{"calibrationDb": -2.3}"#), Some(Tenths(-23)));
        assert_eq!(read(r#"{"calibrationDb": -500}"#), Some(Tenths(-2000)));
        assert_eq!(read(r#"{"calibrationDb": null}"#), None);

        let shoutbox = Shoutbox {
            calibration_db: Some(Tenths(-23)),
            ..Shoutbox::default()
        };
        let json = serde_json::to_string(&shoutbox).expect("writing");
        assert!(json.contains(r#""calibrationDb":-2.3"#), "{json}");
        assert_eq!(shoutbox.control().calibration_db, -2.3);
        assert_eq!(
            Shoutbox::default().control().calibration_db,
            Shoutbox::ESTIMATED_CALIBRATION_DB
        );
    }

    /// An uncalibrated version 2 curve was in weighted dBFS; it comes back in
    /// the estimated SPL it is read in now, at the same place in the room. A
    /// calibrated one was in dB SPL already, and a profile the file had no
    /// shoutbox for gets the default, which is SPL too.
    #[test]
    fn a_version_two_curve_is_lifted_to_spl_once() {
        let file = r#"{"version":2,"active":0,"vocabulary":null,"profiles":[
            {"name":"A","device":"X","channels":[],"groups":[],"outputs":{"shoutbox":{"quietDb":-50}}},
            {"name":"B","device":"X","channels":[],"groups":[],"outputs":{"shoutbox":{"quietDb":80,"calibrationDb":118}}},
            {"name":"C","device":"X","channels":[],"groups":[]}
        ]}"#;
        let path = temp_dir().join(FILE_NAME);
        fs::write(&path, file).expect("writing the version 2 file");

        let config = load(&path);
        let quiet: Vec<i32> = config
            .profiles
            .iter()
            .map(|p| p.outputs.shoutbox.quiet_db)
            .collect();
        assert_eq!(quiet, vec![50, 80, Shoutbox::default().quiet_db]);
        assert_eq!(config.version, VERSION);

        // Saved and read again it is version 3, and nothing moves twice.
        save(&path, &config).expect("saving");
        assert_eq!(load(&path).profiles[0].outputs.shoutbox.quiet_db, 50);
    }

    /// The shoutbox is set up from wherever it is heard (P26); the listen
    /// output stays the host's headphones (P22).
    #[test]
    fn a_remote_may_set_up_the_shoutbox_and_not_the_listen_output() {
        let secret = [3u8; 32];
        let stored = a_config();

        let mut sent = stored.for_remote(&secret);
        let outputs = &mut sent.profiles[1].outputs;
        outputs.shoutbox.output.device_id = "Main PA".into();
        outputs.shoutbox.max_gain_db = 12;
        outputs.listen.output.channels = vec![0, 1];
        outputs.shoutbox.trim_db = -8;
        outputs.shoutbox.muted = true;

        let saved = sent.from_remote(&stored, &secret);
        let kept = &saved.profiles[1].outputs;
        assert_eq!(kept.shoutbox.output.device_id, "Main PA");
        assert_eq!(kept.shoutbox.max_gain_db, 12);
        assert_eq!(kept.listen, stored.profiles[1].outputs.listen);
        assert_eq!(kept.shoutbox.trim_db, -8);
        assert!(kept.shoutbox.muted);
    }

    /// Dropping a webhook is something the remote may do: it is switching a
    /// rule's call off, not writing one.
    #[test]
    fn a_remote_may_drop_a_webhook() {
        let secret = [3u8; 32];
        let stored = with_a_webhook();
        let mut sent = stored.for_remote(&secret);
        sent.profiles[1].rules[1].webhook = None;
        assert_eq!(
            sent.from_remote(&stored, &secret).profiles[1].rules[1].webhook,
            None
        );
    }
}
