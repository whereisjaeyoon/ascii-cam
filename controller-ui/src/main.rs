
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod shared_memory;

use std::{
    sync::mpsc::{self, Receiver, Sender},
    thread::{self, JoinHandle},
    time::Duration,
};

use eframe::egui::{
    self, pos2, vec2, Align2, Color32, FontId, RichText, Sense, Stroke, StrokeKind,
};
use windows::{
    core::{w, Error},
    Win32::{
        Media::MediaFoundation::{
            MFCreateVirtualCamera, MFShutdown, MFStartup, IMFAsyncCallback,
            IMFVirtualCamera, MFSTARTUP_FULL, MF_VERSION,
            MFVirtualCameraAccess_CurrentUser, MFVirtualCameraLifetime_Session,
            MFVirtualCameraType_SoftwareCameraSource,
        },
        System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED},
    },
};

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

    fn wire_value(self) -> u32 {
        match self {
            Self::None => 0,
            Self::Ascii => 1,
            Self::Grayscale => 3,
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

enum CameraCommand {
    Start,
    Stop,
    Shutdown,
}

enum CameraEvent {
    Started(std::result::Result<(), String>),
    Stopped(std::result::Result<(), String>),
}

struct VirtualCameraController {
    command_tx: Sender<CameraCommand>,
    event_rx: Receiver<CameraEvent>,
    worker: Option<JoinHandle<()>>,
}

impl VirtualCameraController {
    fn new() -> Self {
        let (command_tx, command_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();

        let worker = thread::Builder::new()
            .name("ascii-cam-mf".to_owned())
            .spawn(move || {
                camera_worker(command_rx, event_tx);
            })
            .expect("Failed to create camera worker thread");

        Self {
            command_tx,
            event_rx,
            worker: Some(worker),
        }
    }

    fn start(&self) -> std::result::Result<(), String> {
        self.command_tx
            .send(CameraCommand::Start)
            .map_err(|e| e.to_string())
    }

    fn stop(&self) -> std::result::Result<(), String> {
        self.command_tx
            .send(CameraCommand::Stop)
            .map_err(|e| e.to_string())
    }

    fn poll_events(&self) -> impl Iterator<Item = CameraEvent> + '_ {
        self.event_rx.try_iter()
    }
}

impl Drop for VirtualCameraController {
    fn drop(&mut self) {
        let _ = self.command_tx.send(CameraCommand::Shutdown);

        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn camera_worker(
    command_rx: Receiver<CameraCommand>,
    event_tx: Sender<CameraEvent>,
) {
    let com_initialized = unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).is_ok()
    };

    let mut camera: Option<IMFVirtualCamera> = None;
    let mut mf_started = false;

    while let Ok(command) = command_rx.recv() {
        match command {
            CameraCommand::Start => {
                let result = start_virtual_camera(&mut camera, &mut mf_started);
                let _ = event_tx.send(CameraEvent::Started(result));
            }
            CameraCommand::Stop => {
                let result = stop_virtual_camera(&mut camera, &mut mf_started);
                let _ = event_tx.send(CameraEvent::Stopped(result));
            }
            CameraCommand::Shutdown => break,
        }
    }

    let _ = stop_virtual_camera(&mut camera, &mut mf_started);

    if com_initialized {
        unsafe {
            CoUninitialize();
        }
    }
}

fn start_virtual_camera(
    camera: &mut Option<IMFVirtualCamera>,
    mf_started: &mut bool,
) -> std::result::Result<(), String> {
    if camera.is_some() {
        return Ok(());
    }

    unsafe {
        MFStartup(MF_VERSION, MFSTARTUP_FULL)
            .map_err(|e| format!("MFStartup failed: {e}"))?;

        *mf_started = true;

        let vcam = match MFCreateVirtualCamera(
            MFVirtualCameraType_SoftwareCameraSource,
            MFVirtualCameraLifetime_Session,
            MFVirtualCameraAccess_CurrentUser,
            w!("Ascii Cam"),
            w!("{3CAD447D-F283-4AF4-A3B2-6F5363309F52}"),
            None,
        ) {
            Ok(vcam) => vcam,
            Err(error) => {
                let _ = MFShutdown();
                *mf_started = false;

                return Err(format!(
                    "MFCreateVirtualCamera failed: {error}"
                ));
            }
        };

        if let Err(error) = vcam.Start(None::<&IMFAsyncCallback>) {
            let _ = vcam.Stop();
            let _ = vcam.Remove();
            let _ = vcam.Shutdown();
            drop(vcam);

            let _ = MFShutdown();
            *mf_started = false;

            return Err(format!(
                "IMFVirtualCamera::Start failed: {error}"
            ));
        }

        *camera = Some(vcam);
    }

    Ok(())
}

fn stop_virtual_camera(
    camera: &mut Option<IMFVirtualCamera>,
    mf_started: &mut bool,
) -> std::result::Result<(), String> {
    let mut first_error: Option<Error> = None;

    unsafe {
        if let Some(vcam) = camera.take() {
            if let Err(error) = vcam.Stop() {
                first_error = Some(error);
            }

            if let Err(error) = vcam.Remove() {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }

            if let Err(error) = vcam.Shutdown() {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }

            drop(vcam);
        }

        if *mf_started {
            if let Err(error) = MFShutdown() {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }

            *mf_started = false;
        }
    }

    match first_error {
        Some(error) => Err(format!("Failed to stop camera: {error}")),
        None => Ok(()),
    }
}

struct CameraApp {
    enabled: bool,
    camera_pending: bool,
    filter: Filter,
    published: Option<Filter>,
    failed: Option<Filter>,
    publisher: shared_memory::FilterPublisher,
    status: Option<(bool, String)>,
    camera_status: String,
    camera: VirtualCameraController,
}

const PUBLISH_TIMEOUT_MS: u32 = 50;
const RETRY_INTERVAL: Duration = Duration::from_millis(200);

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

        Self {
            enabled: false,
            camera_pending: false,
            filter: Filter::default(),
            published: None,
            failed: None,
            publisher: shared_memory::FilterPublisher::new(),
            status: None,
            camera_status: "Camera is off".to_owned(),
            camera: VirtualCameraController::new(),
        }
    }

    fn publish_filter(&mut self, ctx: &egui::Context) {
        match self
            .publisher
            .publish(self.filter.wire_value(), PUBLISH_TIMEOUT_MS)
        {
            Ok(()) => {
                self.published = Some(self.filter);
                self.status = None;
            }
            Err(e) if e.is_retryable() => {
                self.status = Some((true, e.to_string()));
                ctx.request_repaint_after(RETRY_INTERVAL);
            }
            Err(e) => {
                self.failed = Some(self.filter);
                self.status = Some((false, e.to_string()));
            }
        }
    }

    fn toggle_camera(&mut self) {
        if self.camera_pending {
            return;
        }

        self.camera_pending = true;

        if self.enabled {
            self.camera_status = "Stopping camera...".to_owned();

            if let Err(error) = self.camera.stop() {
                self.camera_pending = false;
                self.camera_status = format!("Failed to stop camera: {error}");
            }
        } else {
            self.camera_status = "Starting camera...".to_owned();

            if let Err(error) = self.camera.start() {
                self.camera_pending = false;
                self.camera_status = format!("Failed to start camera: {error}");
            }
        }
    }

    fn handle_camera_events(&mut self) {
        for event in self.camera.poll_events() {
            match event {
                CameraEvent::Started(Ok(())) => {
                    self.enabled = true;
                    self.camera_pending = false;
                    self.camera_status = "Camera is on".to_owned();
                }
                CameraEvent::Started(Err(error)) => {
                    self.enabled = false;
                    self.camera_pending = false;
                    self.camera_status =
                        format!("Failed to start camera: {error}");
                }
                CameraEvent::Stopped(Ok(())) => {
                    self.enabled = false;
                    self.camera_pending = false;
                    self.camera_status = "Camera is off".to_owned();
                }
                CameraEvent::Stopped(Err(error)) => {
                    self.enabled = false;
                    self.camera_pending = false;
                    self.camera_status = error;
                }
            }
        }
    }

    fn power_switch(&mut self, ui: &mut egui::Ui) {
        let (rect, mut response) =
            ui.allocate_exact_size(vec2(56.0, 32.0), Sense::click());

        if response.clicked() && !self.camera_pending {
            self.toggle_camera();
            response.mark_changed();
        }

        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::Checkbox,
                ui.is_enabled(),
                self.enabled,
                "Virtual camera ON/OFF",
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

        response.on_hover_text("Start or stop the virtual camera.");
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

        let swatch = egui::Rect::from_min_size(
            rect.min + vec2(16.0, 17.0),
            vec2(48.0, 48.0),
        );

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
        self.handle_camera_events();

        if self.camera_pending {
            ctx.request_repaint_after(Duration::from_millis(50));
        }

        if self.published != Some(self.filter)
            && self.failed != Some(self.filter)
        {
            self.publish_filter(ctx);
        }

        egui::TopBottomPanel::top("accent")
            .exact_height(5.0)
            .frame(egui::Frame::new().fill(RED))
            .show(ctx, |_| {});

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(BACKGROUND)
                    .inner_margin(30.0),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("ASCII").size(30.0).strong());
                        ui.label(
                            RichText::new("CAM")
                                .size(30.0)
                                .strong()
                                .color(RED),
                        );

                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                egui::Frame::new()
                                    .fill(Color32::WHITE)
                                    .stroke(Stroke::new(1.0_f32, LINE))
                                    .corner_radius(5.0)
                                    .inner_margin(vec2(9.0, 5.0))
                                    .show(ui, |ui| {
                                        ui.label(
                                            RichText::new("VIRTUAL CAMERA")
                                                .size(10.0)
                                                .color(MUTED),
                                        );
                                    });
                            },
                        );
                    });

                    ui.label(
                        RichText::new("A different way to show up.")
                            .size(14.0)
                            .color(MUTED),
                    );

                    ui.add_space(20.0);
                    ui.separator();
                    ui.add_space(13.0);

                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new("Camera").size(18.0).strong(),
                            );

                            ui.label(
                                RichText::new(if self.camera_pending {
                                    "Camera operation in progress"
                                } else if self.enabled {
                                    "Virtual camera is running"
                                } else {
                                    "Virtual camera is stopped"
                                })
                                .size(12.0)
                                .color(MUTED),
                            );
                        });

                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| self.power_switch(ui),
                        );
                    });

                    ui.label(
                        RichText::new(&self.camera_status)
                            .size(11.0)
                            .color(
                                if self.camera_status.starts_with("Failed") {
                                    RED
                                } else {
                                    MUTED
                                },
                            ),
                    );

                    ui.add_space(13.0);
                    ui.separator();
                    ui.add_space(15.0);

                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Your look").size(18.0).strong(),
                        );

                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                ui.label(
                                    RichText::new("03 FILTERS")
                                        .monospace()
                                        .size(10.0)
                                        .color(MUTED),
                                );
                            },
                        );
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
                        ui.label(
                            RichText::new("Selected").size(12.0).color(MUTED),
                        );
                        ui.label(
                            RichText::new(self.filter.label())
                                .size(12.0)
                                .strong(),
                        );
                    });

                    match &self.status {
                        Some((retrying, message)) => {
                            ui.label(
                                RichText::new(message)
                                    .size(11.0)
                                    .color(if *retrying { MUTED } else { RED }),
                            );
                        }
                        None => {
                            ui.label(
                                RichText::new(match self.published {
                                    Some(filter) => {
                                        format!("Sent: {}", filter.label())
                                    }
                                    None => "Not published yet".to_owned(),
                                })
                                .monospace()
                                .size(11.0)
                                .color(MUTED),
                            );
                        }
                    }
                });
            });
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([540.0, 650.0])
            .with_min_inner_size([500.0, 620.0]),
        ..Default::default()
    };

    eframe::run_native(
        "ASCII Cam",
        options,
        Box::new(|cc| Ok(Box::new(CameraApp::new(cc)))),
    )
}
