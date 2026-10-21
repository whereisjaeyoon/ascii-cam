#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui::{
    self, pos2, vec2, Align2, Color32, FontId, RichText, Sense, Stroke, StrokeKind,
};
use std::sync::mpsc::{self, Receiver};

mod devices;
use devices::CameraDevice;

// UW–Madison Badger Red, with warm neutrals for the rest of the interface.
const RED: Color32 = Color32::from_rgb(197, 5, 12);
const INK: Color32 = Color32::from_rgb(34, 33, 32);
const MUTED: Color32 = Color32::from_rgb(110, 105, 100);
const BACKGROUND: Color32 = Color32::from_rgb(248, 247, 244);
const LINE: Color32 = Color32::from_rgb(225, 222, 216);
const TINT: Color32 = Color32::from_rgb(255, 242, 242);

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Filter {
    #[default]
    None,
    Grayscale,
    Ascii,
}

impl Filter {
    const ALL: [Self; 3] = [Self::None, Self::Grayscale, Self::Ascii];

    fn label(self) -> &'static str {
        match self {
            Self::None => "Original",
            Self::Grayscale => "Grayscale",
            Self::Ascii => "ASCII",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::None => "Natural colors. No filter.",
            Self::Grayscale => "A clean black-and-white look.",
            Self::Ascii => "Light and shadow, drawn in characters.",
        }
    }
}

// Power and filters remain UI-only. Device discovery never starts capture.
#[derive(Default)]
struct CameraApp {
    enabled: bool,
    filter: Filter,
    devices: Vec<CameraDevice>,
    selected_device: Option<String>,
    device_scan: Option<Receiver<Result<Vec<CameraDevice>, String>>>,
    device_error: Option<String>,
}

impl CameraApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut style = (*cc.egui_ctx.style()).clone();
        style.visuals = egui::Visuals::light();
        style.visuals.override_text_color = Some(INK);
        style.visuals.panel_fill = BACKGROUND;
        style.visuals.selection.bg_fill = TINT;
        style.visuals.selection.stroke = Stroke::new(1.0_f32, RED);
        style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, RED);
        style.spacing.item_spacing = vec2(12.0, 10.0);
        style.spacing.button_padding = vec2(16.0, 10.0);
        style
            .text_styles
            .insert(egui::TextStyle::Body, FontId::proportional(14.0));
        style
            .text_styles
            .insert(egui::TextStyle::Button, FontId::proportional(14.0));
        cc.egui_ctx.set_style(style);
        let mut app = Self::default();
        app.refresh_devices(&cc.egui_ctx);
        app
    }

    fn refresh_devices(&mut self, ctx: &egui::Context) {
        if self.device_scan.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let ctx = ctx.clone();
        self.device_error = None;
        self.devices.clear();
        match std::thread::Builder::new()
            .name("camera-discovery".into())
            .spawn(move || {
                let _ = sender.send(devices::enumerate());
                ctx.request_repaint();
            }) {
            Ok(_) => self.device_scan = Some(receiver),
            Err(error) => self.device_error = Some(format!("Could not refresh cameras: {error}")),
        }
    }

    fn poll_devices(&mut self, ctx: &egui::Context) {
        let Some(scan) = &self.device_scan else {
            return;
        };
        let result = match scan.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("Camera discovery stopped. Try Refresh.".into())
            }
        };
        self.device_scan = None;
        match result {
            Ok(devices) => {
                // Use the symbolic link, not the display name: two cameras can
                // have identical names. Preserve selection only while present.
                if !devices
                    .iter()
                    .any(|device| Some(&device.id) == self.selected_device.as_ref())
                {
                    self.selected_device = None;
                }
                self.devices = devices;
            }
            Err(error) => {
                self.selected_device = None;
                self.device_error = Some(error);
            }
        }
    }

    fn device_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Connected devices").size(18.0).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(self.device_scan.is_none(), egui::Button::new("Refresh"))
                    .clicked()
                {
                    self.refresh_devices(ui.ctx());
                }
            });
        });
        ui.label(
            RichText::new("Physical cameras only. Select a device below.")
                .size(12.0)
                .color(MUTED),
        );
        ui.add_space(4.0);
        egui::Frame::new()
            .fill(Color32::WHITE)
            .stroke(Stroke::new(1.0_f32, LINE))
            .corner_radius(10.0)
            .inner_margin(14.0)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                if self.device_scan.is_some() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Looking for cameras...");
                    });
                } else if let Some(error) = &self.device_error {
                    ui.colored_label(RED, error);
                } else if self.devices.is_empty() {
                    ui.label("No physical cameras found.");
                    ui.label(
                        RichText::new("Connect a camera, then choose Refresh.")
                            .size(12.0)
                            .color(MUTED),
                    );
                } else {
                    ui.label(
                        RichText::new(format!("{} camera(s) found", self.devices.len()))
                            .size(11.0)
                            .color(MUTED),
                    );
                    egui::ScrollArea::vertical()
                        .id_salt("device-list")
                        .max_height(145.0)
                        .show(ui, |ui| {
                            for device in &self.devices {
                                ui.push_id(&device.id, |ui| {
                                    let selected =
                                        self.selected_device.as_ref() == Some(&device.id);
                                    if ui.selectable_label(selected, &device.name).clicked() {
                                        self.selected_device = Some(device.id.clone());
                                    }
                                });
                            }
                        });
                }
            });
        ui.label(
            RichText::new("Device selection only. Video capture is not started.")
                .size(11.0)
                .color(MUTED),
        );
    }

    fn power_switch(&mut self, ui: &mut egui::Ui) {
        let (rect, mut response) = ui.allocate_exact_size(vec2(56.0, 32.0), Sense::click());
        if response.clicked() {
            self.enabled = !self.enabled;
            response.mark_changed();
        }
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::Checkbox,
                ui.is_enabled(),
                self.enabled,
                "Camera preview ON/OFF",
            )
        });
        let position = ui.ctx().animate_bool(response.id, self.enabled);
        let fill = if self.enabled {
            RED
        } else {
            Color32::from_rgb(189, 185, 179)
        };
        ui.painter().rect_filled(rect, 16.0, fill);
        ui.painter().circle_filled(
            pos2(rect.left() + 16.0 + position * 24.0, rect.center().y),
            12.0,
            Color32::WHITE,
        );
        if response.has_focus() || response.hovered() {
            ui.painter().rect_stroke(
                rect.expand(3.0),
                19.0,
                Stroke::new(1.0_f32, RED),
                StrokeKind::Outside,
            );
        }
        response.on_hover_text("Preview only. Does not turn on a real camera.");
    }

    fn filter_option(&mut self, ui: &mut egui::Ui, filter: Filter) {
        let (rect, mut response) =
            ui.allocate_exact_size(vec2(ui.available_width(), 82.0), Sense::click());
        if response.clicked() {
            self.filter = filter;
            response.mark_changed();
        }
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::RadioButton,
                ui.is_enabled(),
                self.filter == filter,
                filter.label(),
            )
        });
        let selected = self.filter == filter;
        let border = if selected || response.has_focus() {
            RED
        } else if response.hovered() {
            Color32::from_rgb(175, 168, 161)
        } else {
            LINE
        };
        let fill = if selected { TINT } else { Color32::WHITE };
        ui.painter().rect(
            rect,
            12.0,
            fill,
            Stroke::new(if selected { 1.5_f32 } else { 1.0_f32 }, border),
            StrokeKind::Inside,
        );

        // Small illustrative filter swatches, not a live camera preview.
        let swatch = egui::Rect::from_min_size(rect.min + vec2(16.0, 17.0), vec2(48.0, 48.0));
        ui.painter()
            .rect_filled(swatch, 8.0, Color32::from_rgb(239, 235, 229));
        match filter {
            Filter::None | Filter::Grayscale => {
                let colors = if filter == Filter::None {
                    [
                        Color32::from_rgb(194, 65, 63),
                        Color32::from_rgb(213, 163, 94),
                        Color32::from_rgb(93, 122, 116),
                    ]
                } else {
                    [
                        Color32::from_gray(65),
                        Color32::from_gray(138),
                        Color32::from_gray(203),
                    ]
                };
                for (i, color) in colors.iter().enumerate() {
                    let bar = egui::Rect::from_min_size(
                        swatch.min + vec2(8.0 + i as f32 * 11.0, 9.0),
                        vec2(8.0, 30.0),
                    );
                    ui.painter().rect_filled(bar, 4.0, *color);
                }
            }
            Filter::Ascii => {
                ui.painter().text(
                    swatch.center(),
                    Align2::CENTER_CENTER,
                    "@#\n:.+",
                    FontId::monospace(15.0),
                    INK,
                );
            }
        }
        let text = pos2(swatch.right() + 15.0, rect.top() + 23.0);
        ui.painter().text(
            text,
            Align2::LEFT_CENTER,
            filter.label(),
            FontId::proportional(16.0),
            INK,
        );
        ui.painter().text(
            text + vec2(0.0, 23.0),
            Align2::LEFT_CENTER,
            filter.description(),
            FontId::proportional(12.0),
            MUTED,
        );
        let radio = pos2(rect.right() - 24.0, rect.center().y);
        ui.painter().circle_stroke(
            radio,
            8.0,
            Stroke::new(1.3_f32, if selected { RED } else { LINE }),
        );
        if selected {
            ui.painter().circle_filled(radio, 4.0, RED);
        }
    }
}

impl eframe::App for CameraApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_devices(ctx);
        egui::TopBottomPanel::top("accent")
            .exact_height(5.0)
            .frame(egui::Frame::new().fill(RED))
            .show(ctx, |_| {});
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BACKGROUND).inner_margin(30.0))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("ASCII").size(30.0).strong());
                        ui.label(RichText::new("CAM").size(30.0).strong().color(RED));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            egui::Frame::new()
                                .fill(Color32::WHITE)
                                .stroke(Stroke::new(1.0_f32, LINE))
                                .corner_radius(5.0)
                                .inner_margin(vec2(9.0, 5.0))
                                .show(ui, |ui| {
                                    ui.label(RichText::new("UI PREVIEW").size(10.0).color(MUTED));
                                });
                        });
                    });
                    ui.label(
                        RichText::new("A different way to show up.")
                            .size(14.0)
                            .color(MUTED),
                    );
                    ui.add_space(20.0);
                    ui.separator();
                    ui.add_space(13.0);
                    self.device_panel(ui);
                    ui.add_space(13.0);
                    ui.separator();
                    ui.add_space(13.0);
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new("Camera").size(18.0).strong());
                            ui.label(
                                RichText::new(if self.enabled {
                                    "ON selected in preview"
                                } else {
                                    "OFF selected in preview"
                                })
                                .size(12.0)
                                .color(MUTED),
                            );
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            self.power_switch(ui);
                        });
                    });
                    ui.add_space(13.0);
                    ui.separator();
                    ui.add_space(15.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Your look").size(18.0).strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new("03 FILTERS")
                                    .monospace()
                                    .size(10.0)
                                    .color(MUTED),
                            );
                        });
                    });
                    ui.label(
                        RichText::new("Pick a style. Keep it yours.")
                            .size(12.0)
                            .color(MUTED),
                    );
                    ui.add_space(3.0);
                    for filter in Filter::ALL {
                        self.filter_option(ui, filter);
                    }
                    ui.add_space(14.0);
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Selected").size(12.0).color(MUTED));
                        ui.label(RichText::new(self.filter.label()).size(12.0).strong());
                    });
                    ui.label(
                        RichText::new("Power and filters are UI-only. No video capture.")
                            .size(11.0)
                            .color(MUTED),
                    );
                });
            });
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([540.0, 850.0])
            .with_min_inner_size([500.0, 620.0]),
        ..Default::default()
    };
    eframe::run_native(
        "ASCII Cam",
        options,
        Box::new(|cc| Ok(Box::new(CameraApp::new(cc)))),
    )
}
