use crate::model::{self, Event, Operation};
use eframe::egui::{self, vec2, Align, Color32, FontId, Layout, RichText, Stroke};
use std::sync::mpsc::{self, Receiver};

const RED: Color32 = Color32::from_rgb(197, 5, 12);
const INK: Color32 = Color32::from_rgb(34, 33, 32);
const MUTED: Color32 = Color32::from_rgb(110, 105, 100);
const BACKGROUND: Color32 = Color32::from_rgb(248, 247, 244);
const LINE: Color32 = Color32::from_rgb(225, 222, 216);
const TINT: Color32 = Color32::from_rgb(255, 242, 242);

pub struct SetupApp {
    operation: Operation,
    desktop: bool,
    receiver: Option<Receiver<Event>>,
    progress: f32,
    message: String,
    outcome: Option<Result<String, String>>,
    installed: bool,
    destination: String,
    package_error: Option<String>,
    administrator: bool,
}
impl SetupApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut style = (*cc.egui_ctx.style()).clone();
        style.visuals = egui::Visuals::light();
        style.visuals.override_text_color = Some(INK);
        style.visuals.panel_fill = BACKGROUND;
        style.visuals.selection.bg_fill = TINT;
        style.visuals.selection.stroke = Stroke::new(1.0_f32, RED);
        style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, RED);
        style.spacing.item_spacing = vec2(12.0, 10.0);
        style.spacing.button_padding = vec2(18.0, 12.0);
        style
            .text_styles
            .insert(egui::TextStyle::Body, FontId::proportional(14.0));
        style
            .text_styles
            .insert(egui::TextStyle::Button, FontId::proportional(14.0));
        cc.egui_ctx.set_style(style);
        #[cfg(windows)]
        let (installed, destination, administrator) = (
            crate::windows::installed(),
            crate::windows::directory()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|e| e),
            crate::windows::administrator(),
        );
        #[cfg(not(windows))]
        let (installed, destination, administrator) =
            (false, "C:\\Program Files\\AsciiCam".into(), false);
        Self {
            operation: if std::env::args().any(|a| a == "--uninstall" || a == "uninstall") {
                Operation::Uninstall
            } else {
                Operation::Install
            },
            desktop: false,
            receiver: None,
            progress: 0.0,
            message: String::new(),
            outcome: None,
            installed,
            destination,
            administrator,
            package_error: model::validate_assets(crate::payload::ASSETS).err(),
        }
    }
    fn begin(&mut self, ctx: &egui::Context) {
        let (sender, receiver) = mpsc::channel();
        let operation = self.operation;
        let desktop = self.desktop;
        let ctx = ctx.clone();
        self.outcome = None;
        self.progress = 0.0;
        self.message = "Getting things ready...".into();
        let result = std::thread::Builder::new().name("ascii-cam-install".into()).spawn(move || {
            let result = std::panic::catch_unwind(|| {
                #[cfg(windows)]
                { crate::windows::operate(operation, desktop, crate::payload::ASSETS, &sender) }
                #[cfg(not(windows))]
                { let _ = (operation, desktop); Err("Installation is available on Windows x64 only.".into()) }
            }).unwrap_or_else(|_| Err("The installation worker stopped unexpectedly. Check the installation before retrying.".into()));
            let _ = sender.send(Event::Finished(result));
            ctx.request_repaint();
        });
        match result {
            Ok(_) => self.receiver = Some(receiver),
            Err(error) => {
                self.outcome = Some(Err(format!("Could not start installation: {error}")))
            }
        }
    }
    fn poll(&mut self, ctx: &egui::Context) {
        if let Some(receiver) = &self.receiver {
            let mut finished = false;
            loop {
                match receiver.try_recv() {
                    Ok(Event::Progress { fraction, message }) => {
                        self.progress = fraction;
                        self.message = message;
                    }
                    Ok(Event::Finished(result)) => {
                        if result.is_ok() {
                            self.installed = self.operation == Operation::Install;
                        }
                        self.outcome = Some(result);
                        finished = true;
                        break;
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        self.outcome = Some(Err(
                            "Installer worker disconnected. Check installation before retrying."
                                .into(),
                        ));
                        finished = true;
                        break;
                    }
                }
            }
            if finished {
                self.receiver = None;
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
        }
    }
    fn component(ui: &mut egui::Ui, tag: &str, title: &str, description: &str) {
        egui::Frame::new()
            .fill(Color32::WHITE)
            .stroke(Stroke::new(1.0_f32, LINE))
            .corner_radius(12.0)
            .inner_margin(15.0)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    egui::Frame::new()
                        .fill(TINT)
                        .corner_radius(6.0)
                        .inner_margin(vec2(8.0, 8.0))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(tag)
                                    .monospace()
                                    .size(11.0)
                                    .strong()
                                    .color(RED),
                            );
                        });
                    ui.vertical(|ui| {
                        ui.label(RichText::new(title).size(15.0).strong());
                        ui.label(RichText::new(description).size(12.0).color(MUTED));
                    });
                });
            });
    }
    fn primary(ui: &mut egui::Ui, label: &str, enabled: bool) -> bool {
        ui.add_enabled(
            enabled,
            egui::Button::new(RichText::new(label).color(Color32::WHITE).strong())
                .fill(RED)
                .corner_radius(8.0)
                .min_size(vec2(ui.available_width(), 44.0)),
        )
        .clicked()
    }
    fn install_page(&mut self, ui: &mut egui::Ui) {
        ui.label(
            RichText::new(if self.installed {
                "A fresh start."
            } else {
                "Make every frame yours."
            })
            .size(28.0)
            .strong(),
        );
        ui.label(
            RichText::new(if self.installed {
                "Update your ASCII Cam installation."
            } else {
                "Everything you need, in one installation."
            })
            .size(14.0)
            .color(MUTED),
        );
        ui.add_space(12.0);
        Self::component(
            ui,
            "01",
            "Controller client",
            "Your camera list, power switch and filter controls.",
        );
        Self::component(
            ui,
            "02",
            "Virtual camera host",
            "Start ASCII Cam for use in your camera apps.",
        );
        Self::component(
            ui,
            "03",
            "Camera source DLL",
            "Installed and registered with Windows for you.",
        );
        ui.add_space(8.0);
        ui.label(
            RichText::new("INSTALL LOCATION")
                .monospace()
                .size(10.0)
                .color(MUTED),
        );
        ui.label(RichText::new(&self.destination).size(13.0));
        ui.checkbox(&mut self.desktop, "Add desktop shortcuts");
        ui.add_space(4.0);
        if let Some(error) = &self.package_error {
            ui.colored_label(
                RED,
                "Preview build — installation payload is missing or invalid.",
            );
            ui.label(RichText::new(error).size(11.0).color(MUTED));
        }
        if !self.administrator {
            ui.label(
                RichText::new("Windows administrator permission is required to install.")
                    .size(11.0)
                    .color(MUTED),
            );
        }
        #[cfg(not(windows))]
        ui.colored_label(
            RED,
            "UI preview on this platform. Installation requires Windows x64.",
        );
        if Self::primary(
            ui,
            if self.installed {
                "Update ASCII Cam"
            } else {
                "Install ASCII Cam"
            },
            self.package_error.is_none() && self.administrator && cfg!(windows),
        ) {
            self.begin(ui.ctx());
        }
        ui.label(
            RichText::new("Close Camera, Zoom, OBS and the camera host before installing.")
                .size(11.0)
                .color(MUTED),
        );
    }
    fn remove_page(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Time for a clean slate?").size(28.0).strong());
        ui.label(RichText::new("Remove ASCII Cam from this computer.").color(MUTED));
        ui.add_space(15.0);
        Self::component(
            ui,
            "01",
            "Installed applications",
            "Controller, camera host, DLL and bundled runtimes.",
        );
        Self::component(
            ui,
            "02",
            "Windows registration",
            "Owned camera registration and the Installed apps entry.",
        );
        Self::component(
            ui,
            "03",
            "Application shortcuts",
            "The Start menu and optional desktop shortcuts.",
        );
        ui.add_space(12.0);
        ui.label(RichText::new("Your project source files will stay where they are.").color(MUTED));
        ui.label(RichText::new(&self.destination).size(12.0).color(MUTED));
        ui.add_space(16.0);
        if !self.installed {
            ui.label("ASCII Cam is not registered as installed.");
        }
        if Self::primary(
            ui,
            "Uninstall ASCII Cam",
            self.installed && self.administrator && cfg!(windows),
        ) {
            self.begin(ui.ctx());
        }
        ui.label(
            RichText::new("Close camera apps and both ASCII Cam applications first.")
                .size(11.0)
                .color(MUTED),
        );
    }
    fn result_page(&mut self, ui: &mut egui::Ui) {
        let result = self.outcome.as_ref().unwrap();
        let success = result.is_ok();
        ui.add_space(14.0);
        ui.label(
            RichText::new(if success { "ALL SET" } else { "LET'S FIX THIS" })
                .monospace()
                .size(11.0)
                .color(RED),
        );
        ui.label(
            RichText::new(if success {
                if self.operation == Operation::Install {
                    "You're ready to go."
                } else {
                    "ASCII Cam removed."
                }
            } else {
                "We couldn't finish."
            })
            .size(30.0)
            .strong(),
        );
        ui.add_space(16.0);
        egui::Frame::new()
            .fill(Color32::WHITE)
            .stroke(Stroke::new(1.0_f32, LINE))
            .corner_radius(12.0)
            .inner_margin(20.0)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(match result {
                    Ok(message) | Err(message) => message,
                });
            });
        if success && self.operation == Operation::Install {
            ui.add_space(14.0);
            ui.label(RichText::new("Find these in your Start menu").strong());
            ui.label("ASCII Cam Controller — open the control panel");
            ui.label("ASCII Cam Camera — start the virtual camera host");
            ui.add_space(12.0);
            ui.label(RichText::new("The controller's power and filter controls are still previews; start the camera using its own shortcut.").size(12.0).color(MUTED));
        }
        ui.add_space(24.0);
        if Self::primary(
            ui,
            if success {
                "Close setup"
            } else {
                "Back to setup"
            },
            true,
        ) {
            if success {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                self.outcome = None;
            }
        }
    }
}
impl eframe::App for SetupApp {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.poll(ctx);
        let busy = self.receiver.is_some();
        if busy && ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
        egui::TopBottomPanel::top("accent")
            .exact_height(5.0)
            .frame(egui::Frame::new().fill(RED))
            .show(ctx, |_| {});
        egui::SidePanel::left("brand")
            .exact_width(185.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(Color32::WHITE).inner_margin(24.0))
            .show(ctx, |ui| {
                ui.add_space(12.0);
                ui.label(RichText::new("ASCII").size(27.0).strong());
                ui.label(RichText::new("CAM").size(27.0).strong().color(RED));
                ui.add_space(4.0);
                ui.label(RichText::new("SETUP").monospace().size(11.0).color(MUTED));
                ui.add_space(36.0);
                ui.add_enabled_ui(!busy && self.outcome.is_none(), |ui| {
                    if ui
                        .selectable_label(self.operation == Operation::Install, "Install / Update")
                        .clicked()
                    {
                        self.operation = Operation::Install;
                    }
                    if ui
                        .selectable_label(self.operation == Operation::Uninstall, "Uninstall")
                        .clicked()
                    {
                        self.operation = Operation::Uninstall;
                    }
                });
                ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
                    ui.label(
                        RichText::new(format!("Version {}", model::VERSION))
                            .size(11.0)
                            .color(MUTED),
                    );
                    ui.label(
                        RichText::new("A different way\nto show up.")
                            .size(13.0)
                            .color(MUTED),
                    );
                });
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BACKGROUND).inner_margin(28.0))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_space(9.0);
                    if busy {
                        ui.label(
                            RichText::new(if self.operation == Operation::Install {
                                "Making it yours."
                            } else {
                                "Cleaning things up."
                            })
                            .size(28.0)
                            .strong(),
                        );
                        ui.add_space(20.0);
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(&self.message);
                        });
                        ui.add(
                            egui::ProgressBar::new(self.progress)
                                .fill(RED)
                                .desired_height(8.0),
                        );
                        ui.label(
                            RichText::new(
                                "Please keep this window open until the operation finishes.",
                            )
                            .size(12.0)
                            .color(MUTED),
                        );
                    } else if self.outcome.is_some() {
                        self.result_page(ui);
                    } else if self.operation == Operation::Install {
                        self.install_page(ui);
                    } else {
                        self.remove_page(ui);
                    }
                });
            });
    }
}
