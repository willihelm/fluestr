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
use std::fs;
use std::path::{Path, PathBuf};

/// Schema version of the configuration file.
///
/// Written on every save and checked on every load. A file from a *newer*
/// version is not silently reinterpreted — see [`load`].
pub const VERSION: u32 = 1;

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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
}

/// A named view onto some of the channels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
}

/// One production's setup: which interface, which channels, which views.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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

/// The whole configuration file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
    /// Catalogue id of the speech model (P11), e.g. `small-q5_1`.
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
    /// A `.bin` outside the catalogue. The fallback for a machine that will
    /// never reach the internet: the operator carries the file in and says
    /// where it is. Outranks [`Config::model`] when set.
    #[serde(default)]
    pub model_path: Option<String>,
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
            profiles: Vec::new(),
        }
    }
}

impl Config {
    /// Brings a file that has been edited, truncated or written by another
    /// version into a state the rest of the app can rely on.
    fn sanitise(&mut self) {
        for profile in &mut self.profiles {
            let channels = profile.channels.len();
            for group in &mut profile.groups {
                group.members.retain(|&m| m < channels);
            }
        }

        if self.active >= self.profiles.len() {
            self.active = 0;
        }

        self.version = VERSION;
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
            "intercom2chat-config-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
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
                    }],
                    groups: vec![],
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
                        },
                        StoredChannel {
                            channel: 3,
                            label: "Licht".into(),
                            language: "en".into(),
                            color: "#5fd38d".into(),
                        },
                    ],
                    groups: vec![StoredGroup {
                        name: "Bühne".into(),
                        members: vec![1],
                    }],
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
}
