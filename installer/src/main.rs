mod pe;
#[cfg(windows)]
mod windows;

fn main() -> std::process::ExitCode {
    #[cfg(windows)]
    let result = windows::run();
    #[cfg(not(windows))]
    let result: Result<(), Box<dyn std::error::Error>> =
        Err("This installer runs on 64-bit Windows only.".into());

    let code = match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ERROR: {error}");
            std::process::ExitCode::FAILURE
        }
    };
    // An elevated child has its own console. Keep its result readable.
    if std::env::args_os().any(|arg| arg == "--elevated") {
        println!("Press Enter to close.");
        let _ = std::io::stdin().read_line(&mut String::new());
    }
    code
}
