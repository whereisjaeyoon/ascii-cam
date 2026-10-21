#![cfg_attr(not(windows), allow(dead_code))]
use serde::{Deserialize, Serialize};

pub const CLIENT: &str = "ascii-cam-ui.exe";
pub const HOST: &str = "ascii-cam-host.exe";
pub const DLL: &str = "ascii-cam.dll";
pub const SETUP: &str = "ascii-cam-setup.exe";
pub const MANIFEST: &str = "install-manifest.json";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct Asset {
    pub name: &'static str,
    pub bytes: &'static [u8],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Install,
    Uninstall,
}

#[derive(Debug)]
pub enum Event {
    Progress { fraction: f32, message: String },
    Finished(Result<String, String>),
}

#[derive(Serialize, Deserialize, Debug)]
pub struct InstallManifest {
    pub product: String,
    pub schema: u32,
    pub version: String,
    pub files: Vec<String>,
    pub desktop_shortcuts: bool,
}

pub fn allowed_payload_name(name: &str) -> bool {
    if name.is_empty() || name.contains(['/', '\\', ':']) || name.ends_with([' ', '.']) {
        return false;
    }
    if [CLIENT, HOST, DLL, SETUP].contains(&name) {
        return true;
    }
    name.ends_with(".dll")
        && [
            "vcruntime140",
            "msvcp140",
            "concrt140",
            "vccorlib140",
            "ucrtbase",
            "api-ms-win-crt-",
            "opencv_world",
            "opencv_videoio_",
        ]
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

pub fn validate_manifest(manifest: &InstallManifest) -> Result<(), &'static str> {
    if manifest.product != "ASCII Cam" || manifest.schema != 1 || manifest.files.is_empty() {
        return Err("Invalid installation manifest. No files were removed.");
    }
    let mut names = std::collections::HashSet::new();
    for name in &manifest.files {
        if !allowed_payload_name(name) || !names.insert(name.to_lowercase()) {
            return Err(
                "Unsafe or duplicate file in installation manifest. No files were removed.",
            );
        }
    }
    Ok(())
}

pub fn validate_assets(assets: &[Asset]) -> Result<(), String> {
    for name in [CLIENT, HOST, DLL] {
        if !assets.iter().any(|asset| asset.name == name) {
            return Err(format!("This is a UI preview build. Missing bundled {name}. Build with build-installer.ps1."));
        }
    }
    let mut names = std::collections::HashSet::new();
    for asset in assets {
        if !allowed_payload_name(asset.name)
            || asset.name == SETUP
            || !names.insert(asset.name.to_lowercase())
        {
            return Err(format!("Invalid package file: {}", asset.name));
        }
        if asset.name == DLL {
            crate::pe::validate(asset.bytes).map_err(str::to_owned)?;
        } else {
            crate::pe::validate_image(asset.bytes, asset.name.ends_with(".dll"))
                .map_err(str::to_owned)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uninstall_cannot_escape_install_directory() {
        for name in [
            "../file.dll",
            "C:\\Windows\\file.dll",
            "sub/file.dll",
            "ascii-cam.dll:other",
            "",
            "notes.txt",
            "kernel32.dll",
            "ascii-cam-ui.exe.",
        ] {
            assert!(!allowed_payload_name(name), "{name}");
        }
        for name in [
            CLIENT,
            HOST,
            DLL,
            SETUP,
            "vcruntime140_1.dll",
            "msvcp140.dll",
            "opencv_world4120.dll",
        ] {
            assert!(allowed_payload_name(name), "{name}");
        }
    }
    #[test]
    fn rejects_duplicate_and_foreign_manifest() {
        let mut m = InstallManifest {
            product: "ASCII Cam".into(),
            schema: 1,
            version: "0.2".into(),
            files: vec![DLL.into(), DLL.into()],
            desktop_shortcuts: false,
        };
        assert!(validate_manifest(&m).is_err());
        m.files.pop();
        assert!(validate_manifest(&m).is_ok());
        m.product = "Other App".into();
        assert!(validate_manifest(&m).is_err());
    }
}
