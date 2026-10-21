//! Device discovery only. Never activates a camera or opens a video stream.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CameraDevice {
    pub id: String,
    pub name: String,
}

// Fail closed when a driver does not supply its hardware classification.
// Some virtual drivers identify themselves as hardware, so also reject known
// virtual camera names. This is driver metadata, not a universal proof of hardware.
#[cfg(any(windows, test))]
fn is_physical(hardware: Option<u32>, name: &str, id: &str) -> bool {
    let name = name.to_lowercase();
    hardware.is_some_and(|value| value != 0)
        && ![
            "virtual",
            "ascii cam",
            "ascii-cam",
            "obs camera",
            "manycam",
            "snap camera",
            "xsplit",
            "splitcam",
            "mmhmm",
        ]
        .iter()
        .any(|marker| name.contains(marker))
        && !id
            .to_lowercase()
            .contains("3cad447d-f283-4af4-a3b2-6f5363309f52")
}

#[cfg(not(windows))]
pub fn enumerate() -> Result<Vec<CameraDevice>, String> {
    Err("Camera discovery is available on Windows only.".into())
}

#[cfg(windows)]
pub fn enumerate() -> Result<Vec<CameraDevice>, String> {
    windows_devices::enumerate().map_err(|error| format!("Could not read cameras: {error}"))
}

#[cfg(windows)]
mod windows_devices {
    use super::{is_physical, CameraDevice};
    use windows::{
        core::{Result, GUID, PWSTR},
        Win32::{
            Media::MediaFoundation::*,
            System::Com::{CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_MULTITHREADED},
        },
    };

    struct Com;
    impl Drop for Com {
        fn drop(&mut self) {
            unsafe { CoUninitialize() }
        }
    }

    struct MediaFoundation;
    impl Drop for MediaFoundation {
        fn drop(&mut self) {
            let _ = unsafe { MFShutdown() };
        }
    }

    #[derive(Default)]
    struct Activations {
        ptr: *mut Option<IMFActivate>,
        count: u32,
    }
    impl Drop for Activations {
        fn drop(&mut self) {
            if !self.ptr.is_null() {
                unsafe {
                    for i in 0..self.count as usize {
                        drop((*self.ptr.add(i)).take());
                    }
                    CoTaskMemFree(Some(self.ptr.cast()));
                }
            }
        }
    }

    fn string_attribute(device: &IMFActivate, key: &GUID) -> Result<String> {
        unsafe {
            let mut ptr = PWSTR::null();
            let mut length = 0;
            device.GetAllocatedString(key, &mut ptr, &mut length)?;
            let text = if ptr.is_null() {
                String::new()
            } else {
                String::from_utf16_lossy(std::slice::from_raw_parts(ptr.0, length as usize))
            };
            CoTaskMemFree(Some(ptr.0.cast()));
            Ok(text)
        }
    }

    pub fn enumerate() -> Result<Vec<CameraDevice>> {
        // Called on a fresh worker thread. Every successful initialization has
        // a matching cleanup, including when an enumeration step returns early.
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        }
        let _com = Com;
        unsafe {
            MFStartup(MF_VERSION, MFSTARTUP_LITE)?;
        }
        let _mf = MediaFoundation;
        let mut attributes = None;
        unsafe {
            MFCreateAttributes(&mut attributes, 1)?;
        }
        let attributes =
            attributes.expect("MFCreateAttributes returned success without attributes");
        unsafe {
            attributes.SetGUID(
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
            )?;
        }
        let mut sources = Activations::default();
        unsafe {
            MFEnumDeviceSources(&attributes, &mut sources.ptr, &mut sources.count)?;
        }
        let mut devices = Vec::new();
        for index in 0..sources.count as usize {
            let Some(source) = (unsafe { &*sources.ptr.add(index) }) else {
                continue;
            };
            let hardware = unsafe {
                source
                    .GetUINT32(&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_HW_SOURCE)
                    .ok()
            };
            let Ok(id) = string_attribute(
                source,
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK,
            ) else {
                continue;
            };
            let name = string_attribute(source, &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME)
                .ok()
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| "Camera".into());
            if !id.is_empty() && is_physical(hardware, &name, &id) {
                devices.push(CameraDevice { id, name });
            }
        }
        devices.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then(a.id.cmp(&b.id))
        });
        let mut ids = std::collections::HashSet::new();
        devices.retain(|device| ids.insert(device.id.clone()));
        Ok(devices)
    }
}

#[cfg(test)]
mod tests {
    use super::is_physical;

    #[test]
    fn rejects_software_and_unclassified_sources() {
        assert!(!is_physical(Some(0), "Camera", "device-a"));
        assert!(!is_physical(None, "Camera", "device-a"));
    }

    #[test]
    fn rejects_known_virtual_devices_even_when_marked_hardware() {
        for name in ["Ascii Cam", "OBS Virtual Camera", "OBS Camera", "ManyCam"] {
            assert!(!is_physical(Some(1), name, "device-a"));
        }
        assert!(!is_physical(
            Some(1),
            "Camera",
            "{3CAD447D-F283-4AF4-A3B2-6F5363309F52}"
        ));
    }

    #[test]
    fn accepts_hardware_cameras() {
        for name in ["Integrated Camera", "USB Video Device", "Logitech BRIO"] {
            assert!(is_physical(Some(1), name, "device-a"));
        }
    }
}
