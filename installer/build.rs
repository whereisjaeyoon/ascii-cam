use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=app.manifest");
    println!("cargo:rerun-if-env-changed=ASCII_CAM_PAYLOAD_DIR");
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        assert_eq!(
            env::var("CARGO_CFG_TARGET_ARCH").as_deref(),
            Ok("x86_64"),
            "Build for x86_64 Windows."
        );
        assert_eq!(
            env::var("CARGO_CFG_TARGET_ENV").as_deref(),
            Ok("msvc"),
            "Use the MSVC Rust target."
        );
        println!("cargo:rustc-link-arg-bin=ascii-cam-setup=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg-bin=ascii-cam-setup=/MANIFESTUAC:NO");
        println!(
            "cargo:rustc-link-arg-bin=ascii-cam-setup=/MANIFESTINPUT:{}",
            root.join("app.manifest").display()
        );
    }
    let mut generated = String::from("pub static ASSETS: &[crate::model::Asset] = &[\n");
    if env::var_os("CARGO_FEATURE_BUNDLED").is_some() {
        let payload = env::var_os("ASCII_CAM_PAYLOAD_DIR")
            .map(PathBuf::from)
            .unwrap_or(root.join("payload"));
        let mut files: Vec<_> = fs::read_dir(&payload)
            .expect(
                "Missing payload. Run build-installer.ps1 on Windows to build the distributable.",
            )
            .map(|entry| entry.unwrap().path())
            .collect();
        files.sort();
        for name in ["ascii-cam-ui.exe", "ascii-cam-host.exe", "ascii-cam.dll"] {
            assert!(
                payload.join(name).is_file(),
                "Required payload missing: {name}"
            );
        }
        for path in files {
            assert!(path.is_file(), "Payload must contain only files.");
            let name = path
                .file_name()
                .unwrap()
                .to_str()
                .expect("Payload name must be Unicode.");
            assert!(
                name == "ascii-cam-ui.exe"
                    || name == "ascii-cam-host.exe"
                    || name.ends_with(".dll"),
                "Unexpected payload file: {name}"
            );
            assert!(
                fs::metadata(&path).unwrap().len() > 0,
                "Empty payload: {name}"
            );
            println!("cargo:rerun-if-changed={}", path.display());
            let absolute = path.canonicalize().unwrap();
            generated += &format!(
                "crate::model::Asset {{ name: {name:?}, bytes: include_bytes!({:?}) }},\n",
                absolute.to_str().unwrap()
            );
        }
    }
    generated += "];\n";
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("payload.rs"),
        generated,
    )
    .unwrap();
}
