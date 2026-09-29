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
//! On Windows the equivalent is the WASAPI endpoint id (`IMMDevice::GetId`),
//! stable across reboots and distinct per unit, matched against cpal's list
//! the same way: cpal names a WASAPI device by its friendly name, which is
//! what this reads next to the id (P18).
//!
//! Anything this cannot answer comes back as `None`, and the caller falls back
//! to the name.

/// Which side of a device a list is about.
///
/// Matters for the match, not only for the filter: the catalogue has to hold
/// exactly the devices cpal enumerated, or an output-only interface takes a
/// name away from the input device of the same name (P22, P26).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Input,
    Output,
}

/// A stable id per name in `names`, in the same order.
///
/// `None` means "no stable id for this device" — not an error: the caller has
/// the name and is expected to keep using it.
pub fn assign(names: &[String], direction: Direction) -> Vec<Option<String>> {
    by_name(names, catalogue(direction))
}

/// Every device of one direction the platform can name and identify, in
/// enumeration order, as `(name, stable id)`.
#[cfg(target_os = "macos")]
fn catalogue(direction: Direction) -> Vec<(String, String)> {
    macos::devices(direction)
}

#[cfg(windows)]
fn catalogue(direction: Direction) -> Vec<(String, String)> {
    wasapi::devices(direction)
}

#[cfg(not(any(target_os = "macos", windows)))]
fn catalogue(_: Direction) -> Vec<(String, String)> {
    // No platform id to read: every device falls back to its name, which is
    // what the whole app did before this module existed.
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
        kAudioObjectPropertyScopeInput, kAudioObjectPropertyScopeOutput, kAudioObjectSystemObject,
        AudioObjectGetPropertyData, AudioObjectGetPropertyDataSize, AudioObjectID,
        AudioObjectPropertyAddress, AudioObjectPropertySelector,
    };
    use objc2_core_foundation::{CFRetained, CFString};
    use std::ffi::c_void;
    use std::mem;
    use std::ptr::{null, NonNull};

    use super::Direction;

    /// `(name, UID)` for every device that has at least one stream in
    /// `direction`.
    ///
    /// A device CoreAudio will not name, or will not give a UID for, is left
    /// out entirely rather than half-described: the caller's fallback is the
    /// name it already has from cpal.
    pub(super) fn devices(direction: Direction) -> Vec<(String, String)> {
        let scope = match direction {
            Direction::Input => kAudioObjectPropertyScopeInput,
            Direction::Output => kAudioObjectPropertyScopeOutput,
        };
        device_ids()
            .into_iter()
            .filter(|&id| has_streams(id, scope))
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

    /// Whether the device carries streams in `scope`.
    ///
    /// The device list is every audio object, output-only interfaces
    /// included, and those must not take a name away from the input device
    /// the picker is actually offering — nor an input-only one from an output.
    fn has_streams(device: AudioObjectID, scope: u32) -> bool {
        let address = address(kAudioDevicePropertyStreams, scope);
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

#[cfg(windows)]
mod wasapi {
    use windows::core::BSTR;
    use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
    use windows::Win32::Media::Audio::{
        eCapture, eRender, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
        COINIT_APARTMENTTHREADED, STGM_READ,
    };

    use super::Direction;

    /// `(friendly name, endpoint id)` for every active endpoint in
    /// `direction` — capture for inputs, render for outputs.
    ///
    /// An endpoint that will not give both is left out, as on macOS: the
    /// caller already has its name from cpal.
    pub(super) fn devices(direction: Direction) -> Vec<(String, String)> {
        let _com = Com::init();
        // SAFETY: COM is initialised on this thread for the guard's lifetime,
        // which outlives every interface pointer created below.
        unsafe { endpoints(direction) }.unwrap_or_default()
    }

    unsafe fn endpoints(direction: Direction) -> windows::core::Result<Vec<(String, String)>> {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let flow = match direction {
            Direction::Input => eCapture,
            Direction::Output => eRender,
        };
        let collection = enumerator.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)?;

        let mut devices = Vec::new();
        for index in 0..collection.GetCount()? {
            let Ok(device) = collection.Item(index) else {
                continue;
            };
            if let (Some(name), Some(id)) = (friendly_name(&device), endpoint_id(&device)) {
                devices.push((name, id));
            }
        }
        Ok(devices)
    }

    /// The name cpal gives the same endpoint — it reads this very property.
    unsafe fn friendly_name(device: &IMMDevice) -> Option<String> {
        let store = device.OpenPropertyStore(STGM_READ).ok()?;
        let value = store.GetValue(&PKEY_Device_FriendlyName).ok()?;
        BSTR::try_from(&value).ok().map(|name| name.to_string())
    }

    unsafe fn endpoint_id(device: &IMMDevice) -> Option<String> {
        let id = device.GetId().ok()?;
        let string = id.to_string().ok();
        // The id is allocated by the endpoint and is ours to free.
        CoTaskMemFree(Some(id.0 as *const _));
        string
    }

    /// COM for the current thread, released again only if this call was the
    /// one that initialised it.
    ///
    /// cpal has usually initialised it already on the enumerating thread, as
    /// single-threaded apartment; asking again in the same mode is harmless
    /// and simply answers "already". A thread in the other mode refuses, and
    /// COM is still usable there — so the only case that must not be
    /// balanced with an uninitialise is that refusal.
    struct Com(bool);

    impl Com {
        fn init() -> Self {
            // SAFETY: no reserved pointer; the mode matches cpal's.
            Self(unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok())
        }
    }

    impl Drop for Com {
        fn drop(&mut self) {
            if self.0 {
                // SAFETY: balances the successful initialisation above.
                unsafe { CoUninitialize() };
            }
        }
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

    /// Against the machine's real endpoints, which only a Windows machine has:
    /// `cargo test -p audio-core catalogue_on_this_machine -- --ignored --nocapture`.
    /// Every capture endpoint named, with an id of its own.
    #[cfg(windows)]
    #[test]
    #[ignore = "needs the machine's audio endpoints"]
    fn catalogue_on_this_machine() {
        let devices = super::catalogue(super::Direction::Input);
        for (name, id) in &devices {
            println!("{name:40} {id}");
        }
        let mut ids: Vec<&String> = devices.iter().map(|(_, id)| id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), devices.len(), "two endpoints share an id");
        assert!(devices
            .iter()
            .all(|(name, id)| !name.is_empty() && !id.is_empty()));
    }
}
