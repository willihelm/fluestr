//! Stable device identity.
//!
//! cpal identifies a device by its name and exposes nothing else — its
//! `audio_device_id` is `pub(crate)`. A name is not an identity: two identical
//! interfaces produce the same one, and renaming a device in Audio MIDI Setup
//! breaks every stored reference to it. P12 asks for an id that survives both.
//!
//! On macOS CoreAudio has exactly that in `kAudioDevicePropertyDeviceUID` — a
//! string the driver keeps across reboots, renames and re-plugs, and which
//! differs between two units of the same model. This module reads it and
//! matches it to what cpal enumerated.
//!
//! **How the two lists are matched.** Both walk `kAudioHardwarePropertyDevices`
//! in the order CoreAudio returns it, so the n-th input device here is the
//! n-th input device there. Rather than trusting that alone, the match is made
//! on the name and the position is only the tie-break *among devices sharing a
//! name* — which is the one case a name cannot settle. A distinct name is
//! therefore matched correctly even if the two enumerations ever diverge, and
//! the ambiguous case degrades to what the app does today.
//!
//! Anything this cannot answer comes back as `None`, and the caller falls back
//! to the name. That is the Windows path too, until the WASAPI port brings its
//! own endpoint id.

/// A stable id per name in `names`, in the same order.
///
/// `None` means "no stable id for this device" — not an error: the caller has
/// the name and is expected to keep using it.
pub fn assign(names: &[String]) -> Vec<Option<String>> {
    by_name(names, catalogue())
}

/// Every input device the platform can name and identify, in enumeration
/// order, as `(name, stable id)`.
#[cfg(target_os = "macos")]
fn catalogue() -> Vec<(String, String)> {
    macos::input_devices()
}

#[cfg(not(target_os = "macos"))]
fn catalogue() -> Vec<(String, String)> {
    // Windows (P18): `IMMDevice::GetId` is the equivalent and belongs here. Until then every device falls back to its name, which is what the
    // whole app did before this module existed.
    Vec::new()
}

/// The matching itself, kept free of the platform so it can be tested.
///
/// Consumes each catalogue entry at most once, so the second device called
/// "Scarlett 2i2" gets the second such id and not the first one again.
fn by_name(names: &[String], catalogue: Vec<(String, String)>) -> Vec<Option<String>> {
    let mut unused: Vec<Option<(String, String)>> = catalogue.into_iter().map(Some).collect();

    names
        .iter()
        .map(|name| {
            let slot = unused
                .iter()
                .position(|entry| entry.as_ref().is_some_and(|(n, _)| n == name))?;
            unused[slot].take().map(|(_, id)| id)
        })
        .collect()
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2_core_audio::{
        kAudioDevicePropertyDeviceUID, kAudioDevicePropertyStreams, kAudioHardwarePropertyDevices,
        kAudioObjectPropertyElementMain, kAudioObjectPropertyName, kAudioObjectPropertyScopeGlobal,
        kAudioObjectPropertyScopeInput, kAudioObjectSystemObject, AudioObjectGetPropertyData,
        AudioObjectGetPropertyDataSize, AudioObjectID, AudioObjectPropertyAddress,
        AudioObjectPropertySelector,
    };
    use objc2_core_foundation::{CFRetained, CFString};
    use std::ffi::c_void;
    use std::mem;
    use std::ptr::{null, NonNull};

    /// `(name, UID)` for every device that has at least one input stream.
    ///
    /// A device CoreAudio will not name, or will not give a UID for, is left
    /// out entirely rather than half-described: the caller's fallback is the
    /// name it already has from cpal.
    pub(super) fn input_devices() -> Vec<(String, String)> {
        device_ids()
            .into_iter()
            .filter(|&id| has_input(id))
            .filter_map(|id| {
                let name = string_property(id, kAudioObjectPropertyName)?;
                let uid = string_property(id, kAudioDevicePropertyDeviceUID)?;
                Some((name, uid))
            })
            .collect()
    }

    fn address(selector: AudioObjectPropertySelector, scope: u32) -> AudioObjectPropertyAddress {
        AudioObjectPropertyAddress {
            mSelector: selector,
            mScope: scope,
            mElement: kAudioObjectPropertyElementMain,
        }
    }

    /// Every audio object the system knows, in CoreAudio's own order.
    fn device_ids() -> Vec<AudioObjectID> {
        let address = address(
            kAudioHardwarePropertyDevices,
            kAudioObjectPropertyScopeGlobal,
        );
        let Some(size) = property_size(kAudioObjectSystemObject as AudioObjectID, &address) else {
            return Vec::new();
        };

        let count = size as usize / mem::size_of::<AudioObjectID>();
        let mut ids: Vec<AudioObjectID> = vec![0; count];
        if ids.is_empty() {
            return ids;
        }

        let mut size = size;
        // SAFETY: the buffer holds exactly `size` bytes, which is what the
        // size query above reported, and the address outlives the call.
        let status = unsafe {
            AudioObjectGetPropertyData(
                kAudioObjectSystemObject as AudioObjectID,
                NonNull::from(&address),
                0,
                null(),
                NonNull::from(&mut size),
                NonNull::new(ids.as_mut_ptr()).unwrap().cast::<c_void>(),
            )
        };
        if status != 0 {
            return Vec::new();
        }

        ids.truncate(size as usize / mem::size_of::<AudioObjectID>());
        ids
    }

    /// Whether the device carries input streams.
    ///
    /// The device list is every audio object, output-only interfaces
    /// included, and those must not take a name away from the input device
    /// the picker is actually offering.
    fn has_input(device: AudioObjectID) -> bool {
        let address = address(kAudioDevicePropertyStreams, kAudioObjectPropertyScopeInput);
        property_size(device, &address).is_some_and(|size| size > 0)
    }

    /// A `CFString`-valued property, as an owned `String`.
    fn string_property(
        device: AudioObjectID,
        selector: AudioObjectPropertySelector,
    ) -> Option<String> {
        let address = address(selector, kAudioObjectPropertyScopeGlobal);
        let mut value: *const CFString = null();
        let mut size = mem::size_of::<*const CFString>() as u32;

        // SAFETY: `value` is a single pointer and `size` says so; CoreAudio
        // writes at most that much. The address outlives the call.
        let status = unsafe {
            AudioObjectGetPropertyData(
                device,
                NonNull::from(&address),
                0,
                null(),
                NonNull::from(&mut size),
                NonNull::from(&mut value).cast::<c_void>(),
            )
        };
        if status != 0 {
            return None;
        }

        let value = NonNull::new(value.cast_mut())?;
        // SAFETY: the property follows the Copy rule and hands out a +1
        // reference, which `CFRetained` now owns and releases on drop.
        let string = unsafe { CFRetained::from_raw(value) };
        Some(string.to_string())
    }

    /// `None` when the object has no such property, which is how a device
    /// without input streams is told apart from one with them.
    fn property_size(object: AudioObjectID, address: &AudioObjectPropertyAddress) -> Option<u32> {
        let mut size = 0u32;
        // SAFETY: both pointers are to live locals and outlive the call.
        let status = unsafe {
            AudioObjectGetPropertyDataSize(
                object,
                NonNull::from(address),
                0,
                null(),
                NonNull::from(&mut size),
            )
        };
        (status == 0).then_some(size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn catalogue(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(n, u)| (n.to_string(), u.to_string()))
            .collect()
    }

    #[test]
    fn a_device_is_matched_by_name() {
        let ids = by_name(
            &names(&["Scarlett 2i2", "MacBook Pro Microphone"]),
            catalogue(&[
                ("MacBook Pro Microphone", "BuiltInMicrophoneDevice"),
                ("Scarlett 2i2", "AppleUSBAudioEngine:Focus:USB:1"),
            ]),
        );

        assert_eq!(
            ids,
            vec![
                Some("AppleUSBAudioEngine:Focus:USB:1".to_string()),
                Some("BuiltInMicrophoneDevice".to_string()),
            ]
        );
    }

    #[test]
    fn two_identical_interfaces_get_different_ids() {
        // The case the whole module exists for: the name settles nothing, so
        // the enumeration order is the tie-break, and neither device may be
        // handed the other's id.
        let ids = by_name(
            &names(&["Scarlett 2i2", "Scarlett 2i2"]),
            catalogue(&[("Scarlett 2i2", "...USB:1"), ("Scarlett 2i2", "...USB:2")]),
        );

        assert_eq!(
            ids,
            vec![Some("...USB:1".to_string()), Some("...USB:2".to_string())]
        );
    }

    #[test]
    fn a_device_the_platform_did_not_describe_has_no_id() {
        // Not an error. The caller keeps using the name, which is what the
        // app did before stable ids existed.
        let ids = by_name(
            &names(&["Scarlett 2i2", "Dante Virtual Soundcard"]),
            catalogue(&[("Scarlett 2i2", "...USB:1")]),
        );

        assert_eq!(ids, vec![Some("...USB:1".to_string()), None]);
    }

    #[test]
    fn a_catalogue_entry_nobody_asked_for_is_ignored() {
        let ids = by_name(
            &names(&["Scarlett 2i2"]),
            catalogue(&[
                ("Loopback Audio", "com.rogueamoeba.Loopback:0"),
                ("Scarlett 2i2", "...USB:1"),
            ]),
        );

        assert_eq!(ids, vec![Some("...USB:1".to_string())]);
    }
}
