use crate::{
    model::{self, Asset, Event, InstallManifest, Operation, CLIENT, DLL, HOST, MANIFEST, SETUP},
    transaction::{reject_link, FileTransaction},
};
use std::{
    error::Error,
    fs, io,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::mpsc::Sender,
};
use windows::{
    core::{Interface, PCWSTR},
    Win32::{
        Storage::FileSystem::{MoveFileExW, MOVEFILE_DELAY_UNTIL_REBOOT},
        System::Com::{
            CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, IPersistFile,
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
        },
        UI::Shell::{
            FOLDERID_CommonPrograms, FOLDERID_ProgramFilesX64, FOLDERID_PublicDesktop, IShellLinkW,
            IsUserAnAdmin, SHGetKnownFolderPath, ShellLink, KF_FLAG_DEFAULT,
        },
    },
};
use winreg::{enums::*, RegKey, RegValue};

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
const COM: &str = r"SOFTWARE\Classes\CLSID\{3CAD447D-F283-4AF4-A3B2-6F5363309F52}\InprocServer32";
const CLSID: &str = r"SOFTWARE\Classes\CLSID\{3CAD447D-F283-4AF4-A3B2-6F5363309F52}";
const APP: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\AsciiCam";
const APP_VALUES: &[&str] = &[
    "DisplayName",
    "DisplayVersion",
    "InstallLocation",
    "UninstallString",
    "DisplayIcon",
    "NoModify",
    "NoRepair",
    "EstimatedSize",
    "InstallerKind",
];

// Machine-wide installation is serialized across simultaneous Setup windows.
struct SetupMutex(windows::Win32::Foundation::HANDLE);
impl SetupMutex {
    fn acquire() -> Result<Self> {
        use windows::Win32::{
            Foundation::{CloseHandle, WAIT_ABANDONED, WAIT_OBJECT_0},
            System::Threading::{CreateMutexW, WaitForSingleObject},
        };
        let name: Vec<u16> = "Global\\AsciiCam.Setup.3CAD447D-F283-4AF4-A3B2-6F5363309F52"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        unsafe {
            let handle = CreateMutexW(None, false, PCWSTR(name.as_ptr()))?;
            let status = WaitForSingleObject(handle, 0);
            if status != WAIT_OBJECT_0 && status != WAIT_ABANDONED {
                let _ = CloseHandle(handle);
                return Err("Another ASCII Cam installation or removal is running.".into());
            }
            Ok(Self(handle))
        }
    }
}
impl Drop for SetupMutex {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::System::Threading::ReleaseMutex(self.0);
            let _ = windows::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

struct Com;
impl Com {
    fn initialize() -> Result<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        }
        Ok(Self)
    }
}
impl Drop for Com {
    fn drop(&mut self) {
        unsafe { CoUninitialize() }
    }
}
fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}
fn string(path: &Path) -> Result<String> {
    Ok(path
        .to_str()
        .ok_or("Installation path is not Unicode")?
        .into())
}
fn known_folder(id: &windows::core::GUID) -> Result<PathBuf> {
    unsafe {
        let text = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None)?;
        let converted = text.to_string();
        CoTaskMemFree(Some(text.0.cast()));
        Ok(PathBuf::from(converted?))
    }
}
pub fn directory() -> std::result::Result<PathBuf, String> {
    known_folder(&FOLDERID_ProgramFilesX64)
        .map(|p| p.join("AsciiCam"))
        .map_err(|e| e.to_string())
}
pub fn installed() -> bool {
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags(APP, KEY_READ | KEY_WOW64_64KEY)
        .is_ok()
}
pub fn administrator() -> bool {
    unsafe { IsUserAnAdmin().as_bool() }
}
fn progress(tx: &Sender<Event>, fraction: f32, message: &str) {
    let _ = tx.send(Event::Progress {
        fraction,
        message: message.into(),
    });
}

#[derive(Default)]
struct RegistryTransaction {
    values: Vec<(String, String, Option<RegValue>)>,
    created: Vec<String>,
    finished: bool,
}
impl RegistryTransaction {
    fn key(&mut self, path: &str) -> Result<RegKey> {
        let root = RegKey::predef(HKEY_LOCAL_MACHINE);
        let mut prefix = String::new();
        for segment in path.split('\\') {
            if !prefix.is_empty() {
                prefix.push('\\');
            }
            prefix.push_str(segment);
            match root.open_subkey_with_flags(&prefix, KEY_READ | KEY_WOW64_64KEY) {
                Ok(_) => (),
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    root.create_subkey_with_flags(&prefix, KEY_READ | KEY_WRITE | KEY_WOW64_64KEY)?;
                    self.created.push(prefix.clone());
                }
                Err(e) => return Err(e.into()),
            }
        }
        Ok(root.open_subkey_with_flags(path, KEY_READ | KEY_WRITE | KEY_WOW64_64KEY)?)
    }
    fn change(&mut self, path: &str, name: &str, value: Option<RegValue>) -> Result<()> {
        let key = self.key(path)?;
        let before = match key.get_raw_value(name) {
            Ok(v) => Some(v),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        if let Some(v) = value {
            key.set_raw_value(name, &v)?;
        } else if before.is_some() {
            key.delete_value(name)?;
        }
        self.values.push((path.into(), name.into(), before));
        Ok(())
    }
    fn text(&mut self, path: &str, name: &str, value: &str) -> Result<()> {
        use winreg::types::ToRegValue;
        self.change(path, name, Some(value.to_reg_value()))
    }
    fn number(&mut self, path: &str, name: &str, value: u32) -> Result<()> {
        use winreg::types::ToRegValue;
        self.change(path, name, Some(value.to_reg_value()))
    }
    fn commit(&mut self) {
        self.finished = true;
        self.values.clear();
        self.created.clear();
    }
    fn rollback(&mut self) -> Vec<String> {
        let root = RegKey::predef(HKEY_LOCAL_MACHINE);
        let mut failures = Vec::new();
        for (path, name, before) in self.values.drain(..).rev() {
            let result: io::Result<()> = (|| {
                let (key, _) =
                    root.create_subkey_with_flags(&path, KEY_READ | KEY_WRITE | KEY_WOW64_64KEY)?;
                if let Some(value) = before {
                    key.set_raw_value(&name, &value)
                } else {
                    match key.delete_value(&name) {
                        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
                        r => r,
                    }
                }
            })();
            if let Err(e) = result {
                failures.push(format!("Registry rollback failed for {path}: {e}"));
            }
        }
        for path in self.created.drain(..).rev() {
            remove_empty_key(&path);
        }
        self.finished = true;
        failures
    }
}
impl Drop for RegistryTransaction {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.rollback();
        }
    }
}
fn remove_empty_key(path: &str) {
    let root = RegKey::predef(HKEY_LOCAL_MACHINE);
    if let Ok(key) = root.open_subkey_with_flags(path, KEY_READ | KEY_WOW64_64KEY) {
        let empty = key.enum_keys().next().is_none() && key.enum_values().next().is_none();
        drop(key);
        if empty {
            let _ = root.delete_subkey(path);
        }
    }
}
fn same_path(a: &Path, b: &Path) -> bool {
    a.to_string_lossy()
        .eq_ignore_ascii_case(&b.to_string_lossy())
}
fn shortcut_paths(desktop: bool) -> Result<Vec<(PathBuf, &'static str)>> {
    let mut locations = vec![known_folder(&FOLDERID_CommonPrograms)?];
    if desktop {
        locations.push(known_folder(&FOLDERID_PublicDesktop)?);
    }
    let mut result = Vec::new();
    for directory in locations {
        reject_link(&directory)?;
        result.push((directory.join("ASCII Cam Controller.lnk"), CLIENT));
        result.push((directory.join("ASCII Cam Camera.lnk"), HOST));
    }
    Ok(result)
}
fn shortcut_bytes(target: &Path, directory: &Path) -> Result<Vec<u8>> {
    // Generate into a private temporary directory, then install via the same
    // rollback transaction as the other files. All COM work stays on this worker.
    let temp = std::env::temp_dir().join(format!(
        "ascii-link-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    fs::create_dir(&temp)?;
    let file = temp.join("shortcut.lnk");
    let outcome = (|| -> Result<Vec<u8>> {
        unsafe {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            let target_w = wide(target);
            let directory_w = wide(directory);
            link.SetPath(PCWSTR(target_w.as_ptr()))?;
            link.SetWorkingDirectory(PCWSTR(directory_w.as_ptr()))?;
            link.SetIconLocation(PCWSTR(target_w.as_ptr()), 0)?;
            let persist: IPersistFile = link.cast()?;
            let file_w = wide(&file);
            persist.Save(PCWSTR(file_w.as_ptr()), true)?;
        }
        Ok(fs::read(&file)?)
    })();
    let _ = fs::remove_file(file);
    let _ = fs::remove_dir(temp);
    outcome
}
fn manifest(dir: &Path) -> Result<Option<InstallManifest>> {
    let file = dir.join(MANIFEST);
    reject_link(&file)?;
    match fs::read(file) {
        Ok(bytes) => {
            let value: InstallManifest = serde_json::from_slice(&bytes)?;
            model::validate_manifest(&value)?;
            Ok(Some(value))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn operate(
    operation: Operation,
    desktop: bool,
    assets: &[Asset],
    sender: &Sender<Event>,
) -> std::result::Result<String, String> {
    let result = (|| -> Result<String> {
        if !cfg!(target_arch = "x86_64") {
            return Err("Use the x64 installer.".into());
        }
        if !administrator() {
            return Err("Administrator permission is required. Right-click Setup and choose Run as administrator.".into());
        }
        let _mutex = SetupMutex::acquire()?;
        let _com = Com::initialize()?;
        if operation == Operation::Install {
            let key = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey_with_flags(
                r"SOFTWARE\Microsoft\Windows NT\CurrentVersion",
                KEY_READ | KEY_WOW64_64KEY,
            )?;
            let build: String = key.get_value("CurrentBuildNumber")?;
            if build.parse::<u32>()? < 22000 {
                return Err("ASCII Cam requires Windows 11 (build 22000 or later).".into());
            }
        }
        let dir = directory()?;
        reject_link(&dir)?;
        if operation == Operation::Install {
            model::validate_assets(assets)?;
        }
        let dir_existed = dir.exists();
        if operation == Operation::Install {
            fs::create_dir_all(&dir)?;
        }
        let mut files = FileTransaction::default();
        let mut registry = RegistryTransaction::default();
        let result = match operation {
            Operation::Install => install(&dir, desktop, assets, sender, &mut files, &mut registry),
            Operation::Uninstall => uninstall(&dir, sender, &mut files, &mut registry),
        };
        match result {
            Ok(message) => {
                files.commit();
                registry.commit();
                if operation == Operation::Uninstall {
                    remove_empty_key(COM);
                    remove_empty_key(CLSID);
                    remove_empty_key(APP);
                    let cleanup = cleanup_installer(&dir);
                    let _ = fs::remove_dir(&dir); // Never recursive; preserve unknown files.
                    return Ok(format!("{message}{cleanup}"));
                }
                Ok(message)
            }
            Err(error) => {
                let mut failures = registry.rollback();
                failures.extend(files.rollback());
                if !dir_existed {
                    let _ = fs::remove_dir(&dir);
                }
                if failures.is_empty() {
                    Err(format!("{error}\n\nChanges from this attempt were rolled back. Close Camera, Zoom, OBS and the camera host before retrying. If the DLL remains locked, restart Windows and install before opening camera apps.").into())
                } else {
                    Err(format!(
                        "{error}\n\nRollback needs attention:\n{}",
                        failures.join("\n")
                    )
                    .into())
                }
            }
        }
    })();
    result.map_err(|error| error.to_string())
}

fn install(
    dir: &Path,
    desktop: bool,
    assets: &[Asset],
    tx: &Sender<Event>,
    files: &mut FileTransaction,
    registry: &mut RegistryTransaction,
) -> Result<String> {
    progress(tx, 0.08, "Checking the installation");
    let old_manifest = manifest(dir)?;
    let current = std::env::current_exe()?;
    let setup = dir.join(SETUP);
    let mut names: Vec<String> = assets.iter().map(|a| a.name.to_owned()).collect();
    names.push(SETUP.into());
    // Prepare shortcut data before modifying installed files.
    let links = shortcut_paths(desktop)?;
    let link_data = links
        .iter()
        .map(|(path, name)| Ok((path.clone(), shortcut_bytes(&dir.join(name), dir)?)))
        .collect::<Result<Vec<_>>>()?;
    for (i, asset) in assets.iter().enumerate() {
        let message = if asset.name == CLIENT {
            "Installing the controller"
        } else if asset.name == HOST {
            "Installing the camera host"
        } else if asset.name == DLL {
            "Installing the camera source"
        } else {
            "Installing runtime libraries"
        };
        progress(tx, 0.16 + (i as f32 / assets.len() as f32) * 0.42, message);
        files
            .write(&dir.join(asset.name), asset.bytes)
            .map_err(|e| format!("Could not replace {}: {e}", asset.name))?;
    }
    // Do not replace the executable currently running from the installation dir.
    if !same_path(&current, &setup) {
        files.write(&setup, &fs::read(&current)?)?;
    }
    if let Some(previous) = &old_manifest {
        for name in &previous.files {
            if !names.contains(name) {
                files.remove(&dir.join(name))?;
            }
        }
        if previous.desktop_shortcuts && !desktop {
            let old_links = shortcut_paths(true)?;
            for (path, _) in old_links.into_iter().skip(2) {
                files.remove(&path)?;
            }
        }
    }
    progress(tx, 0.70, "Registering the camera with Windows");
    registry.text(COM, "", &string(&dir.join(DLL))?)?;
    registry.text(COM, "ThreadingModel", "Both")?;
    registry.text(APP, "DisplayName", "ASCII Cam")?;
    registry.text(APP, "DisplayVersion", model::VERSION)?;
    registry.text(APP, "InstallLocation", &string(dir)?)?;
    registry.text(
        APP,
        "UninstallString",
        &format!("\"{}\" --uninstall", string(&setup)?),
    )?;
    registry.text(
        APP,
        "DisplayIcon",
        &format!("\"{}\",0", string(&dir.join(CLIENT))?),
    )?;
    registry.text(APP, "InstallerKind", "AsciiCamRustSetup")?;
    registry.number(APP, "NoModify", 1)?;
    registry.number(APP, "NoRepair", 1)?;
    let total: u64 =
        assets.iter().map(|a| a.bytes.len() as u64).sum::<u64>() + fs::metadata(&current)?.len();
    registry.number(
        APP,
        "EstimatedSize",
        (total / 1024).min(u32::MAX as u64) as u32,
    )?;
    progress(tx, 0.87, "Adding your shortcuts");
    for (path, bytes) in link_data {
        files.write(&path, &bytes)?;
    }
    let installed = InstallManifest {
        product: "ASCII Cam".into(),
        schema: 1,
        version: model::VERSION.into(),
        files: names,
        desktop_shortcuts: desktop,
    };
    files.write(&dir.join(MANIFEST), &serde_json::to_vec_pretty(&installed)?)?;
    progress(tx, 1.0, "Installation complete");
    Ok("Client, camera host and DLL installed. Camera registration and Start menu shortcuts are ready.".into())
}

fn uninstall(
    dir: &Path,
    tx: &Sender<Event>,
    files: &mut FileTransaction,
    registry: &mut RegistryTransaction,
) -> Result<String> {
    progress(tx, 0.08, "Checking installed components");
    let root = RegKey::predef(HKEY_LOCAL_MACHINE);
    let app = root.open_subkey_with_flags(APP, KEY_READ | KEY_WOW64_64KEY)?;
    let location: String = app.get_value("InstallLocation")?;
    if !same_path(Path::new(&location), dir) {
        return Err("Unexpected installation path. No files were removed.".into());
    }
    drop(app);
    // Compatibility with the earlier DLL-only installer: it had no manifest.
    let installed = manifest(dir)?;
    let names = installed
        .as_ref()
        .map(|m| m.files.clone())
        .unwrap_or_else(|| vec![DLL.into(), SETUP.into()]);
    let mut owned_com = false;
    match root.open_subkey_with_flags(COM, KEY_READ | KEY_WOW64_64KEY) {
        Ok(key) => {
            let path: String = key.get_value("")?;
            owned_com = same_path(Path::new(&path), &dir.join(DLL));
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => (),
        Err(e) => return Err(e.into()),
    }
    progress(tx, 0.22, "Removing installed files");
    for name in &names {
        if name != SETUP {
            files
                .remove(&dir.join(name))
                .map_err(|e| format!("Could not remove {name}: {e}"))?;
        }
    }
    if let Some(m) = &installed {
        for (path, _) in shortcut_paths(m.desktop_shortcuts)? {
            files.remove(&path)?;
        }
    }
    files.remove(&dir.join(MANIFEST))?;
    progress(tx, 0.72, "Removing camera registration");
    // Preserve a COM registration that another installation now owns.
    if owned_com {
        registry.change(COM, "", None)?;
        registry.change(COM, "ThreadingModel", None)?;
    }
    for name in APP_VALUES {
        registry.change(APP, name, None)?;
    }
    progress(tx, 1.0, "Uninstall complete");
    Ok("Installed components, owned camera registration and shortcuts were removed. Project source files were preserved.".into())
}

fn cleanup_installer(dir: &Path) -> String {
    let setup = dir.join(SETUP);
    if !setup.exists() {
        return String::new();
    }
    if reject_link(&setup).is_err() {
        return " The remaining Setup file needs manual removal.".into();
    }
    if fs::remove_file(&setup).is_ok() {
        return String::new();
    }
    // Never schedule deletion of the stable setup path: reinstalling before a
    // reboot would otherwise cause the NEW installer to be deleted on reboot.
    let tombstone = dir.join(format!(
        "ascii-cam-setup.removed-{}-{}.exe",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    if fs::rename(&setup, &tombstone).is_ok() {
        let path = wide(&tombstone);
        if unsafe {
            MoveFileExW(
                PCWSTR(path.as_ptr()),
                PCWSTR::null(),
                MOVEFILE_DELAY_UNTIL_REBOOT,
            )
        }
        .is_ok()
        {
            return " The uninstaller file will be removed after Windows restarts.".into();
        }
        return format!(
            " Close this window, then delete the remaining file: {}",
            tombstone.display()
        );
    }
    " Close this window, then remove the remaining ascii-cam-setup.exe from the installation folder.".into()
}
