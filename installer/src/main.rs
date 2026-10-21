#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod model;
mod pe;
mod transaction;
mod ui;
#[cfg(windows)]
mod windows;
mod payload {
    include!(concat!(env!("OUT_DIR"), "/payload.rs"));
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([760.0, 700.0])
            .with_min_inner_size([710.0, 650.0]),
        ..Default::default()
    };
    eframe::run_native(
        "ASCII Cam Setup",
        options,
        Box::new(|cc| Ok(Box::new(ui::SetupApp::new(cc)))),
    )
}
