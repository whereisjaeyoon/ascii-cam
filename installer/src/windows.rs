use std::{
    error::Error,
    ffi::{OsStr, OsString},
    fs, io,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
};
use windows_sys::Win32::{
    Storage::FileSystem::{MoveFileExW, MOVEFILE_DELAY_UNTIL_REBOOT},
    UI::{
        Shell::{IsUserAnAdmin, ShellExecuteW},
        WindowsAndMessaging::SW_SHOWNORMAL,
    },
};
use winreg::{enums::*, RegKey, RegValue};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const CLSID: &str = r"SOFTWARE\Classes\CLSID\{3CAD447D-F283-4AF4-A3B2-6F5363309F52}";
const COM: &str = r"SOFTWARE\Classes\CLSID\{3CAD447D-F283-4AF4-A3B2-6F5363309F52}\InprocServer32";
const APP: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\AsciiCam";
const SETUP: &str = "ascii-cam-setup.exe";

fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(Some(0)).collect()
}
fn text(path: &Path) -> Result<String> {
    Ok(path.to_str().ok_or("Path is not valid Unicode")?.to_owned())
}
fn quoted(value: &OsStr) -> OsString {
    // Windows command-line quoting: double backslashes before quotes and at end.
    let mut result = OsString::from("\"");
    let mut slashes = 0;
    for ch in value.to_string_lossy().chars() {
        if ch == '\\' {
            slashes += 1;
            continue;
        }
        if ch == '"' {
            result.push("\\".repeat(slashes * 2 + 1));
        } else {
            result.push("\\".repeat(slashes));
        }
        result.push(ch.to_string());
        slashes = 0;
    }
    result.push("\\".repeat(slashes * 2));
    result.push("\"");
    result
}
fn elevate(args: &[OsString]) -> Result<()> {
    let exe = wide(std::env::current_exe()?.as_os_str());
    let cwd = wide(std::env::current_dir()?.as_os_str());
    let mut command = OsString::new();
    for arg in args {
        command.push(quoted(arg));
        command.push(" ");
    }
    command.push("--elevated");
    let args = wide(&command);
    let verb = wide(OsStr::new("runas"));
    // SAFETY: all UTF-16 strings are NUL terminated and live through this call.
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            exe.as_ptr(),
            args.as_ptr(),
            cwd.as_ptr(),
            SW_SHOWNORMAL,
        )
    } as isize;
    if result <= 32 {
        return Err(format!(
            "Administrator permission was not granted (ShellExecute code {result})."
        )
        .into());
    }
    println!("Opened the administrator installer window. Check that window for the result.");
    Ok(())
}

fn directory() -> Result<PathBuf> {
    let base = std::env::var_os("ProgramW6432")
        .or_else(|| std::env::var_os("ProgramFiles"))
        .ok_or("Program Files location not found")?;
    Ok(PathBuf::from(base).join("AsciiCam"))
}

type Snapshot = Option<Vec<(String, RegValue)>>;
fn snapshot(path: &str) -> Result<Snapshot> {
    match RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags(path, KEY_READ | KEY_WOW64_64KEY)
    {
        Ok(key) => Ok(Some(key.enum_values().collect::<io::Result<Vec<_>>>()?)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
fn restore(path: &str, values: &Snapshot) -> Result<()> {
    let root = RegKey::predef(HKEY_LOCAL_MACHINE);
    match root.delete_subkey_all(path) {
        Ok(()) => (),
        Err(e) if e.kind() == io::ErrorKind::NotFound => (),
        Err(e) => return Err(e.into()),
    }
    if let Some(values) = values {
        let (key, _) = root.create_subkey_with_flags(path, KEY_WRITE | KEY_WOW64_64KEY)?;
        for (name, value) in values {
            key.set_raw_value(name, value)?;
        }
    }
    Ok(())
}
fn file_snapshot(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
fn replace_file(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let temporary = path.with_extension(format!("install-{}.tmp", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let result: io::Result<()> = (|| {
        file.write_all(bytes)?;
        file.sync_all()
    })();
    drop(file);
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    Ok(())
}
fn restore_file(path: &Path, bytes: &Option<Vec<u8>>) -> Result<()> {
    if let Some(bytes) = bytes {
        replace_file(path, bytes)?;
    } else if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn install(source: &Path) -> Result<()> {
    let bytes = fs::read(source).map_err(|e| format!("Cannot read {}: {e}", source.display()))?;
    super::pe::validate(&bytes)?;
    let dir = directory()?;
    let dll = dir.join("ascii-cam.dll");
    let setup = dir.join(SETUP);
    let this_exe = std::env::current_exe()?.canonicalize()?;
    let copy_setup = !setup.exists() || setup.canonicalize()? != this_exe;
    let prior_com = snapshot(COM)?;
    let prior_app = snapshot(APP)?;
    let prior_dll = file_snapshot(&dll)?;
    let prior_setup = if copy_setup {
        file_snapshot(&setup)?
    } else {
        None
    };
    fs::create_dir_all(&dir)?;
    let mut dll_written = false;
    let mut setup_written = false;
    let mut registry_changed = false;
    let outcome: Result<()> = (|| {
        // No DLL is loaded here. An in-use DLL fails with an actionable error.
        replace_file(&dll,&bytes).map_err(|e| format!("Cannot replace {}: {e}. Close Camera/Zoom/OBS and the camera host; reboot if it remains in use.",dll.display()))?;
        dll_written = true;
        if copy_setup {
            replace_file(&setup, &fs::read(&this_exe)?)?;
            setup_written = true;
        }
        registry_changed = true;
        let root = RegKey::predef(HKEY_LOCAL_MACHINE);
        let (key, _) = root.create_subkey_with_flags(COM, KEY_WRITE | KEY_WOW64_64KEY)?;
        // Matches the inspected camera DLL's DllRegisterServer implementation.
        key.set_value("", &text(&dll)?)?;
        key.set_value("ThreadingModel", &"Both")?;
        let (app, _) = root.create_subkey_with_flags(APP, KEY_WRITE | KEY_WOW64_64KEY)?;
        app.set_value("DisplayName", &"ASCII Cam")?;
        app.set_value("DisplayVersion", &env!("CARGO_PKG_VERSION"))?;
        app.set_value("InstallLocation", &text(&dir)?)?;
        app.set_value(
            "UninstallString",
            &format!("\"{}\" uninstall", text(&setup)?),
        )?;
        app.set_value("NoModify", &1u32)?;
        app.set_value("NoRepair", &1u32)?;
        Ok(())
    })();
    if let Err(error) = outcome {
        // Report rollback failures instead of pretending installation was atomic.
        if registry_changed {
            for (path, values) in [(COM, &prior_com), (APP, &prior_app)] {
                if let Err(e) = restore(path, values) {
                    eprintln!("Registry rollback failed for {path}: {e}");
                }
            }
        }
        if dll_written {
            if let Err(e) = restore_file(&dll, &prior_dll) {
                eprintln!("DLL rollback failed: {e}");
            }
        }
        if setup_written {
            if let Err(e) = restore_file(&setup, &prior_setup) {
                eprintln!("Installer rollback failed: {e}");
            }
        }
        return Err(error);
    }
    println!("Installed DLL: {}", dll.display());
    println!("COM registration: HKLM\\{COM}");
    println!("Installed apps entry: ASCII Cam");
    println!("Installation complete. This does not start the camera or connect the UI.");
    Ok(())
}

fn uninstall() -> Result<()> {
    let root = RegKey::predef(HKEY_LOCAL_MACHINE);
    let app = root.open_subkey_with_flags(APP, KEY_READ | KEY_WOW64_64KEY)?;
    let dir = directory()?;
    let stored: String = app.get_value("InstallLocation")?;
    if !stored.eq_ignore_ascii_case(&text(&dir)?) {
        return Err("Unexpected installation path; refusing to delete files.".into());
    }
    drop(app);
    let dll = dir.join("ascii-cam.dll");
    let setup = dir.join(SETUP);
    let prior_dll = file_snapshot(&dll)?;
    let prior_com = snapshot(COM)?;
    let prior_app = snapshot(APP)?;
    let own_com = match root.open_subkey_with_flags(COM, KEY_READ | KEY_WOW64_64KEY) {
        Ok(key) => key
            .get_value::<String, _>("")?
            .eq_ignore_ascii_case(&text(&dll)?),
        Err(e) if e.kind() == io::ErrorKind::NotFound => false,
        Err(e) => return Err(e.into()),
    };
    if dll.exists() {
        fs::remove_file(&dll).map_err(|e| {
            format!(
                "Cannot remove DLL: {e}. Close camera applications and camera host, then retry."
            )
        })?;
    }
    let result: Result<()> = (|| {
        if own_com {
            root.delete_subkey_all(CLSID)?;
        }
        root.delete_subkey_all(APP)?;
        Ok(())
    })();
    if let Err(error) = result {
        for (path, values) in [(COM, &prior_com), (APP, &prior_app)] {
            if let Err(e) = restore(path, values) {
                eprintln!("Registry rollback failed: {e}");
            }
        }
        if let Err(e) = restore_file(&dll, &prior_dll) {
            eprintln!("DLL rollback failed: {e}");
        }
        return Err(error);
    }
    if setup.exists() && fs::remove_file(&setup).is_err() {
        // Windows cannot delete an executable while it is running. Delete only
        // this known installed file on reboot; never recursively remove a folder.
        let path = wide(setup.as_os_str());
        if unsafe { MoveFileExW(path.as_ptr(), std::ptr::null(), MOVEFILE_DELAY_UNTIL_REBOOT) } == 0
        {
            eprintln!(
                "Remove remaining installer manually after it exits: {}",
                setup.display()
            );
        } else {
            println!("The running uninstaller will be removed after Windows restarts.");
        }
    }
    let _ = fs::remove_dir(&dir); // Only removes an empty directory.
    println!("Uninstalled ASCII Cam. UI source files and projects were preserved.");
    Ok(())
}

fn status() -> Result<()> {
    let root = RegKey::predef(HKEY_LOCAL_MACHINE);
    let key = root.open_subkey_with_flags(COM, KEY_READ | KEY_WOW64_64KEY)?;
    let path: String = key.get_value("")?;
    let threading: String = key.get_value("ThreadingModel")?;
    println!(
        "Registered DLL: {path}\nThreadingModel: {threading}\nFile exists: {}",
        Path::new(&path).is_file()
    );
    println!(
        "Installed-app entry exists: {}",
        root.open_subkey_with_flags(APP, KEY_READ | KEY_WOW64_64KEY)
            .is_ok()
    );
    Ok(())
}

pub fn run() -> Result<()> {
    if !cfg!(target_arch = "x86_64") {
        return Err("Build this installer for x86_64-pc-windows-msvc.".into());
    }
    let args: Vec<OsString> = std::env::args_os()
        .skip(1)
        .filter(|s| s != "--elevated")
        .collect();
    let Some(command) = args.first().and_then(|s| s.to_str()) else {
        println!("ASCII Cam Setup\n\n  ascii-cam-setup.exe install <path-to-ascii-cam.dll>\n  ascii-cam-setup.exe status\n  ascii-cam-setup.exe uninstall\n\nInstall/uninstall request administrator permission. Close camera apps first.");
        return Ok(());
    };
    if command == "status" && args.len() == 1 {
        return status();
    }
    if !((command == "install" && args.len() == 2) || (command == "uninstall" && args.len() == 1)) {
        return Err("Use install <DLL path>, status, or uninstall.".into());
    }
    if unsafe { IsUserAnAdmin() } == 0 {
        if std::env::args_os().any(|arg| arg == "--elevated") {
            return Err("Administrator access is still unavailable.".into());
        }
        return elevate(&args);
    }
    match command {
        "install" => install(Path::new(&args[1])),
        "uninstall" => uninstall(),
        _ => unreachable!(),
    }
}
