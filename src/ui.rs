use eframe::egui::{
    self, Align2, Color32, FontId, Pos2, Rect, RichText, Stroke, Vec2, ViewportCommand,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use shard_typer::{
    engine::{self, Command, SessionState, Snapshot, WorkerEvent},
    platform::{self, HotkeyCommand, HotkeyEvent, HotkeyService},
    settings::{self, ActivationMode, AppSettings, NewlineMode},
    text::{PreparedText, decode_text},
    timing::{CurveModel, DistributionKind, TimingConfig, required_wpm, target_feasible},
};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::design::{self, ERROR, ICE, INK, MUTED};

#[derive(Default, Clone)]
pub struct PreviewOptions {
    pub preview: bool,
    pub timing: bool,
    pub pill: bool,
    pub controls: bool,
    pub small: bool,
    pub target: bool,
    pub screenshot: Option<PathBuf>,
}
impl PreviewOptions {
    pub fn from_args() -> Self {
        let mut p = Self::default();
        let mut args = std::env::args().skip(1);
        while let Some(a) = args.next() {
            match a.as_str() {
                "--preview" => p.preview = true,
                "--timing" => p.timing = true,
                "--pill" => p.pill = true,
                "--controls" => p.controls = true,
                "--small" => p.small = true,
                "--target" => p.target = true,
                "--screenshot" => p.screenshot = args.next().map(PathBuf::from),
                _ => {}
            }
        }
        if p.screenshot.is_some() {
            p.preview = true;
        }
        p
    }
}
struct Pulse {
    ms: f64,
    born: Instant,
}
#[derive(Clone, Copy)]
enum Icon {
    Close,
    Minimize,
    Pin,
    Collapse,
    Expand,
    Play,
    Pause,
    Stop,
    Settings,
}

pub struct ShardApp {
    settings: AppSettings,
    path: PathBuf,
    last_valid: TimingConfig,
    model: CurveModel,
    worker: Option<engine::Worker>,
    hotkeys: Option<HotkeyService>,
    registered: bool,
    status: Snapshot,
    prepared: Arc<PreparedText>,
    pulses: VecDeque<Pulse>,
    last_ms: Option<f64>,
    notice: Option<String>,
    timing_error: Option<String>,
    hotkey_error: Option<String>,
    controls_open: bool,
    hotkey_draft: platform::HotkeySpec,
    native_hwnd: Option<isize>,
    window_shape: Option<[u32; 3]>,
    dirty_at: Option<Instant>,
    broken_settings: bool,
    escape_active: bool,
    preview: PreviewOptions,
    frames: usize,
    created_at: Instant,
    screenshot_requested: bool,
}

impl ShardApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        settings: AppSettings,
        path: PathBuf,
        notice: Option<String>,
        preview: PreviewOptions,
    ) -> Self {
        style(&cc.egui_ctx);
        if let Some(window) = cc.winit_window() {
            window.set_theme(Some(winit::window::Theme::Dark));
        }
        if let Some(position) = settings.position
            && let Some(window) = cc.winit_window()
        {
            let size = window.outer_size();
            let visible =
                platform::visible_position(position, [size.width as f32, size.height as f32]);
            window.set_outer_position(winit::dpi::PhysicalPosition::new(
                visible[0] as i32,
                visible[1] as i32,
            ));
        }
        let native_hwnd = cc.window_handle().ok().and_then(|h| match h.as_raw() {
            RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
            _ => None,
        });
        if let Some(hwnd) = native_hwnd {
            platform::apply_glass(hwnd, settings.blur);
        }
        let ctx = cc.egui_ctx.clone();
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(move || ctx.request_repaint());
        let worker = (!preview.preview)
            .then(|| engine::Worker::spawn(settings.timing.clone(), wake.clone()));
        let hotkeys = worker
            .as_ref()
            .map(|w| HotkeyService::spawn(settings.hotkey.clone(), w.tx.clone(), wake));
        let mut status = Snapshot::default();
        let prepared = Arc::new(PreparedText::new(&settings.text));
        if preview.preview && preview.timing {
            status.state = SessionState::Typing;
            status.position = 28;
            status.total = prepared.len();
            status.elapsed = 6.8;
        }
        let pulses = if preview.preview && preview.timing {
            [180., 271., 222.]
                .into_iter()
                .map(|ms| Pulse {
                    ms,
                    born: Instant::now(),
                })
                .collect()
        } else {
            VecDeque::new()
        };
        Self {
            last_valid: settings.timing.clone(),
            model: settings.timing.model(),
            hotkey_draft: settings.hotkey.clone(),
            settings,
            path,
            worker,
            hotkeys,
            registered: false,
            status,
            prepared,
            pulses,
            last_ms: None,
            timing_error: None,
            hotkey_error: None,
            controls_open: preview.controls,
            native_hwnd,
            window_shape: None,
            dirty_at: None,
            broken_settings: notice.is_some(),
            notice,
            escape_active: false,
            preview,
            frames: 0,
            created_at: Instant::now(),
            screenshot_requested: false,
        }
    }
    fn send(&self, command: Command) {
        if let Some(w) = &self.worker {
            let _ = w.tx.send(command);
        }
    }
    fn start(&mut self, delay: Duration) {
        if let Some(error) = &self.timing_error {
            self.notice = Some(error.clone());
            return;
        }
        self.prepared = Arc::new(PreparedText::new(&self.settings.text));
        if self.prepared.is_empty() {
            self.notice = Some("Add some text before starting.".into());
            return;
        }
        self.pulses.clear();
        self.last_ms = None;
        self.send(Command::Start(
            self.prepared.clone(),
            self.settings.newline,
            delay,
        ));
        self.status = Snapshot {
            state: if delay.is_zero() {
                SessionState::WaitingForKeys
            } else {
                SessionState::Countdown
            },
            total: self.prepared.len(),
            countdown: delay.as_secs_f64(),
            ..Default::default()
        };
        self.enable_escape(true);
    }
    fn enable_escape(&mut self, active: bool) {
        if active != self.escape_active {
            self.escape_active = active;
            if let Some(h) = &self.hotkeys {
                let _ = h.tx.send(HotkeyCommand::Escape(active));
            }
        }
    }
    fn toggle(&mut self, hotkey: bool) {
        match self.status.state {
            SessionState::Typing | SessionState::Countdown | SessionState::WaitingForKeys => {
                self.send(Command::Pause)
            }
            SessionState::Paused => self.send(Command::Resume(if hotkey {
                Duration::ZERO
            } else {
                Duration::from_secs_f64(self.settings.startup_seconds)
            })),
            _ => {
                if hotkey && self.settings.mode != ActivationMode::Hotkey {
                    return;
                }
                self.start(if hotkey {
                    Duration::ZERO
                } else {
                    Duration::from_secs_f64(self.settings.startup_seconds)
                });
            }
        }
    }
    fn stop(&mut self) {
        self.send(Command::Stop);
        self.status = Snapshot::default();
        self.enable_escape(false);
        self.pulses.clear();
        self.last_ms = None;
    }
    fn collapse(&mut self, ctx: &egui::Context) {
        self.settings.collapsed = !self.settings.collapsed;
        ctx.send_viewport_cmd(ViewportCommand::Resizable(!self.settings.collapsed));
        ctx.send_viewport_cmd(ViewportCommand::MinInnerSize(if self.settings.collapsed {
            Vec2::new(400., 72.)
        } else {
            Vec2::new(400., 640.)
        }));
        ctx.send_viewport_cmd(ViewportCommand::InnerSize(if self.settings.collapsed {
            Vec2::new(self.settings.expanded_size[0], 72.)
        } else {
            self.settings.expanded_size.into()
        }));
    }
    fn pin(&mut self, ctx: &egui::Context) {
        self.settings.pinned = !self.settings.pinned;
        ctx.send_viewport_cmd(ViewportCommand::WindowLevel(if self.settings.pinned {
            egui::WindowLevel::AlwaysOnTop
        } else {
            egui::WindowLevel::Normal
        }));
    }
    fn persist(&mut self) {
        if self.preview.preview {
            return;
        }
        if self.broken_settings && self.path.exists() {
            if let Err(e) =
                std::fs::copy(&self.path, self.path.with_file_name("settings.broken.json"))
            {
                self.notice = Some(format!("Could not preserve the original settings: {e}"));
                return;
            }
            self.broken_settings = false;
        }
        let mut saved = self.settings.clone();
        saved.timing = self.last_valid.clone();
        match settings::save(&self.path, &saved) {
            Ok(()) => self.dirty_at = None,
            Err(e) => {
                self.notice = Some(format!("Local settings could not be saved: {e}"));
                self.dirty_at = None;
            }
        }
    }
    fn drain(&mut self) {
        let events: Vec<_> = self
            .worker
            .as_ref()
            .map(|w| w.rx.try_iter().collect())
            .unwrap_or_default();
        for event in events {
            match event {
                WorkerEvent::Status(s) => {
                    self.status = s;
                    self.enable_escape(self.status.state.active());
                }
                WorkerEvent::Character(e) => {
                    self.status.position = e.index + 1;
                    if let Some(ms) = e.actual_ms {
                        self.last_ms = Some(ms);
                        self.pulses.push_back(Pulse {
                            ms,
                            born: Instant::now(),
                        });
                        while self.pulses.len() > 64 {
                            self.pulses.pop_front();
                        }
                    }
                }
            }
        }
        let events: Vec<_> = self
            .hotkeys
            .as_ref()
            .map(|h| h.rx.try_iter().collect())
            .unwrap_or_default();
        for event in events {
            match event {
                HotkeyEvent::Toggle => self.toggle(true),
                HotkeyEvent::Stopped => self.stop(),
                HotkeyEvent::Registered(Ok(spec)) => {
                    self.registered = true;
                    self.settings.hotkey = spec;
                    self.hotkey_error = None;
                    self.dirty_at = Some(Instant::now());
                }
                HotkeyEvent::Registered(Err(e)) => {
                    self.hotkey_error = Some(e);
                }
                HotkeyEvent::EscapeError(e) => self.notice = Some(e),
            }
        }
    }
    fn header(&mut self, ui: &mut egui::Ui) {
        let collapsed = self.settings.collapsed;
        let (rect, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 44.), egui::Sense::hover());
        let buttons_left = rect.right() - (5. * design::ICON_HIT + 4. * design::ICON_GAP);
        let text_rect = Rect::from_min_max(
            rect.min + Vec2::new(30., 0.),
            Pos2::new(buttons_left - 10., rect.bottom()),
        );
        let drag_rect = Rect::from_min_max(rect.min, Pos2::new(buttons_left - 8., rect.bottom()));
        let drag = ui.interact(drag_rect, ui.id().with("chrome-drag"), egui::Sense::drag());
        if drag.drag_started() {
            ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
        }
        shard(
            ui.painter(),
            Rect::from_min_size(rect.min + Vec2::new(0., 3.), Vec2::splat(22.)),
            ICE,
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(text_rect), |ui| {
            if collapsed {
                let preview = self.prepared.preview(
                    if self.status.state.active() || self.status.state == SessionState::Completed {
                        self.status.position
                    } else {
                        0
                    },
                );
                let label = if preview.is_empty() {
                    if self.status.state == SessionState::Completed {
                        "All words delivered"
                    } else {
                        "Your next words…"
                    }
                } else {
                    &preview
                };
                ui.add(egui::Label::new(RichText::new(label).color(INK).size(12.)).truncate());
                let status = match self.status.state {
                    SessionState::Typing => "TYPING",
                    SessionState::Paused => "PAUSED",
                    SessionState::Countdown => "COUNTDOWN",
                    SessionState::Completed => "COMPLETE",
                    _ => "",
                };
                let progress = if status.is_empty() {
                    format!("{} characters", self.prepared.len())
                } else {
                    format!(
                        "{status} · {} / {}",
                        self.status.position,
                        self.prepared.len()
                    )
                };
                ui.label(RichText::new(progress).size(9.).color(MUTED));
            } else {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("shard").size(18.).strong().color(INK));
                    ui.label(RichText::new("typer").size(18.).color(MUTED));
                });
            }
        });
        let buttons = [
            if collapsed {
                (
                    if matches!(
                        self.status.state,
                        SessionState::Typing
                            | SessionState::Countdown
                            | SessionState::WaitingForKeys
                    ) {
                        Icon::Pause
                    } else {
                        Icon::Play
                    },
                    "Start / pause / resume",
                    false,
                )
            } else {
                (Icon::Settings, "Settings", self.controls_open)
            },
            (Icon::Pin, "Keep above other windows", self.settings.pinned),
            (
                if collapsed {
                    Icon::Expand
                } else {
                    Icon::Collapse
                },
                if collapsed {
                    "Expand window"
                } else {
                    "Collapse to preview"
                },
                false,
            ),
            (
                if collapsed {
                    Icon::Stop
                } else {
                    Icon::Minimize
                },
                if collapsed {
                    "Stop and reset"
                } else {
                    "Minimize"
                },
                false,
            ),
            (Icon::Close, "Close Shard Typer", false),
        ];
        for (slot, (icon, label, selected)) in buttons.into_iter().enumerate() {
            // Identical slots, width and top inset in both layouts: no mouse chase.
            let button_rect = Rect::from_min_size(
                Pos2::new(
                    buttons_left + slot as f32 * (design::ICON_HIT + design::ICON_GAP),
                    rect.top(),
                ),
                Vec2::splat(design::ICON_HIT),
            );
            let mut clicked = false;
            ui.scope_builder(egui::UiBuilder::new().max_rect(button_rect), |ui| {
                clicked = icon_button(ui, icon, label, selected);
            });
            if clicked {
                match slot {
                    0 if collapsed => self.toggle(false),
                    0 => {
                        self.controls_open = !self.controls_open;
                        ui.ctx().request_repaint();
                    }
                    1 => self.pin(ui.ctx()),
                    2 => self.collapse(ui.ctx()),
                    3 if collapsed => self.stop(),
                    3 => ui.ctx().send_viewport_cmd(ViewportCommand::Minimized(true)),
                    _ => {
                        self.send(Command::Stop);
                        ui.ctx().send_viewport_cmd(ViewportCommand::Close);
                    }
                }
            }
        }
        ui.advance_cursor_after_rect(rect);
    }
    fn text_panel(&mut self, ui: &mut egui::Ui) {
        let frozen = self.status.state.active();
        ui.horizontal(|ui| {
            fixed_label(ui, "Your text", 88., 30., design::heading_font(), INK);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled_ui(!frozen, |ui| {
                    if quiet_button(ui, "Clear").clicked() {
                        self.settings.text.clear();
                    }
                    if quiet_button(ui, "Import").clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .add_filter("Plain text", &["txt"])
                            .pick_file()
                    {
                        match std::fs::read(path)
                            .map_err(|e| e.to_string())
                            .and_then(|b| decode_text(&b))
                        {
                            Ok(text) => self.settings.text = text,
                            Err(e) => self.notice = Some(e),
                        }
                    }
                    if quiet_button(ui, "Paste").clicked() {
                        match platform::paste_text() {
                            Ok(text) => self.settings.text = text,
                            Err(e) => self.notice = Some(e),
                        }
                    }
                });
            });
        });
        ui.add_space(5.);
        let height = (ui.available_height() - 26.).max(32.);
        card().inner_margin(10).show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("text-scroll")
                .auto_shrink([false, false])
                .max_height((height - 20.).max(16.))
                .min_scrolled_height((height - 20.).max(16.))
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut self.settings.text)
                            .interactive(!frozen)
                            .desired_width(f32::INFINITY)
                            .desired_rows(3)
                            .font(FontId::proportional(14.))
                            .text_color(INK)
                            .frame(egui::Frame::NONE)
                            .hint_text("Paste your text here…")
                            .lock_focus(true),
                    );
                });
        });
        if !frozen {
            self.prepared = Arc::new(PreparedText::new(&self.settings.text));
        }
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!(
                    "{} characters · {} words",
                    self.prepared.len(),
                    self.prepared.text.split_whitespace().count()
                ))
                .size(10.)
                .color(MUTED),
            );
            if frozen {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new("Locked for this run").size(10.).color(MUTED));
                });
            }
        });
    }
    fn split_panels(&mut self, ui: &mut egui::Ui) {
        let rect = ui.available_rect_before_wrap();
        let total = rect.height();
        let top_height = (total * self.settings.text_split).clamp(95., (total - 115.).max(95.));
        let handle = Rect::from_min_size(
            Pos2::new(rect.left(), rect.top() + top_height),
            Vec2::new(rect.width(), 14.),
        );
        let response = ui
            .interact(
                handle,
                ui.id().with("text-timing-divider"),
                egui::Sense::drag(),
            )
            .on_hover_cursor(egui::CursorIcon::ResizeVertical)
            .on_hover_text("Drag to give text or timing more room. Double-click to reset.");
        if response.dragged()
            && let Some(pos) = response.interact_pointer_pos()
        {
            self.settings.text_split = ((pos.y - rect.top()) / total).clamp(0.12, 0.78);
            ui.ctx().request_repaint();
        }
        if response.double_clicked() {
            self.settings.text_split = 0.28;
        }
        let color = if response.hovered() || response.dragged() {
            ICE
        } else {
            Color32::from_white_alpha(60)
        };
        ui.painter().line_segment(
            [
                Pos2::new(handle.left(), handle.center().y),
                Pos2::new(handle.right(), handle.center().y),
            ],
            Stroke::new(1., design::DIVIDER),
        );
        ui.painter().line_segment(
            [
                handle.center() - Vec2::new(18., 0.),
                handle.center() + Vec2::new(18., 0.),
            ],
            Stroke::new(3., color),
        );
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(Rect::from_min_max(
                rect.min,
                Pos2::new(rect.right(), handle.top() - 3.),
            )),
            |ui| centered_group(ui, |ui| self.text_panel(ui)),
        );
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(Rect::from_min_max(
                Pos2::new(rect.left(), handle.bottom() + 3.),
                rect.max,
            )),
            |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("timing-scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.timing_panel(ui));
            },
        );
        ui.allocate_rect(rect, egui::Sense::hover());
    }
    fn metrics(&self, ui: &mut egui::Ui) {
        let characters = if self.status.state.active() {
            self.status.total.saturating_sub(self.status.position)
        } else {
            self.prepared.len()
        };
        ui.columns(3, |cols| {
            metric(
                &mut cols[0],
                "PACE",
                &format!("{:.1}", self.model.wpm()),
                "WPM",
            );
            metric(
                &mut cols[1],
                if self.status.state.active() {
                    "REMAINING"
                } else {
                    "TYPING TIME"
                },
                &duration(self.model.seconds(characters)),
                "",
            );
            metric(
                &mut cols[2],
                "AVERAGE GAP",
                &format!("{:.0}", self.model.mean_ms),
                "ms",
            );
        });
    }
    fn timing_panel(&mut self, ui: &mut egui::Ui) {
        centered_group(ui, |ui| {
            ui.spacing_mut().item_spacing = Vec2::new(8., 6.);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Delay distribution").font(design::heading_font()));
                if let Some(ms) = self.last_ms {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(format!("{ms:.0} ms")).size(11.).color(MUTED))
                            .on_hover_text("Most recent measured character interval");
                    });
                }
            });
            self.graph(ui);
            centered_at(ui, 192., |ui| {
                let normal = self.settings.timing.kind == DistributionKind::Shaped;
                if let Some(index) =
                    segmented(ui, &["Normal", "Uniform"], usize::from(!normal), 192.)
                {
                    self.settings.timing.kind = if index == 0 {
                        DistributionKind::Shaped
                    } else {
                        DistributionKind::Uniform
                    };
                }
            });
            ui.add_space(4.);
            ui.columns(2, |columns| {
                columns[0].horizontal(|ui| {
                    ui.label(RichText::new("Min").size(12.).color(MUTED));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        number_field(
                            ui,
                            &mut self.settings.timing.min_ms,
                            10. ..=60_000.,
                            " ms",
                            design::FIELD_WIDTH,
                        );
                    });
                });
                columns[1].horizontal(|ui| {
                    ui.label(RichText::new("Max").size(12.).color(MUTED));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        number_field(
                            ui,
                            &mut self.settings.timing.max_ms,
                            10. ..=60_000.,
                            " ms",
                            design::FIELD_WIDTH,
                        );
                    });
                });
            });
            ui.add_space(2.);
            let min = self.settings.timing.min_ms;
            let max = self.settings.timing.max_ms.max(min);
            ui.add_enabled_ui(self.settings.timing.kind == DistributionKind::Shaped, |ui| {
                slider_row(ui,"Center",&mut self.settings.timing.center_ms,min..=max," ms",
                    "Underlying location. Skew and bounds change the final average.", Some((min+max)/2.));
                slider_row(ui,"Deviation",&mut self.settings.timing.deviation_ms,0. ..=(max-min)," ms",
                    "Underlying standard deviation. Higher values widen and flatten the curve.", None);
                slider_row(ui,"Skew",&mut self.settings.timing.skew,-10. ..=10.,"",
                    "Negative favors shorter gaps; positive favors longer gaps. Bounds truncate the distribution.", None);
            });
            if let Some(error) = &self.timing_error {
                ui.label(RichText::new(error).size(11.).color(ERROR));
            }
            if self
                .last_ms
                .is_some_and(|ms| ms < self.last_valid.min_ms || ms > self.last_valid.max_ms)
            {
                ui.label(
                    RichText::new("Measured interval outside sampled bounds (scheduler timing).")
                        .size(11.)
                        .color(MUTED),
                );
            }
        });
    }
    fn graph(&mut self, ui: &mut egui::Ui) {
        let (rect, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 90.), egui::Sense::hover());
        let p = ui.painter_at(rect);
        let curve_width = (rect.width() - 138.).clamp(130., 270.);
        let plot = Rect::from_min_max(
            Pos2::new(rect.center().x - curve_width / 2., rect.top() + 10.),
            Pos2::new(rect.center().x + curve_width / 2., rect.bottom() - 24.),
        );
        let min = self.last_valid.min_ms;
        let max = self.last_valid.max_ms;
        let span = (max - min).max(1.);
        let peak = self
            .model
            .points
            .iter()
            .map(|p| p.1)
            .fold(0., f64::max)
            .max(f64::MIN_POSITIVE);
        let reference_density = 1.35 / (span * 0.175 * (2. * std::f64::consts::PI).sqrt());
        let density_scale = peak.max(reference_density);
        let x = |ms: f64| plot.left() + ((ms - min) / span).clamp(0., 1.) as f32 * plot.width();
        let y = |d: f64| plot.bottom() - (d / density_scale) as f32 * plot.height() * 0.92;
        let density_at = |ms: f64| {
            let i = self.model.points.partition_point(|p| p.0 < ms);
            let a = self.model.points[i.saturating_sub(1)];
            let b = self.model.points[i.min(self.model.points.len() - 1)];
            let t = if b.0 > a.0 {
                ((ms - a.0) / (b.0 - a.0)).clamp(0., 1.)
            } else {
                0.
            };
            a.1 + t * (b.1 - a.1)
        };
        p.line_segment(
            [plot.left_bottom(), plot.right_bottom()],
            Stroke::new(1., design::DIVIDER),
        );
        let mean_x = if min == max {
            plot.center().x
        } else {
            x(self.model.mean_ms)
        };
        let mean_y = if self.model.points.len() == 1 {
            plot.top() + 4.
        } else {
            y(density_at(self.model.mean_ms))
        };
        p.line_segment(
            [Pos2::new(mean_x, plot.bottom()), Pos2::new(mean_x, mean_y)],
            Stroke::new(1.2, Color32::from_rgba_unmultiplied(227, 244, 252, 75)),
        );
        if self.model.points.len() == 1 {
            crate::glow::stroke(
                &p,
                &[Pos2::new(mean_x, plot.bottom()), Pos2::new(mean_x, mean_y)],
                ICE,
                design::CURVE_STROKE,
                design::CURVE_HALO,
            );
        } else {
            let points: Vec<_> = self
                .model
                .points
                .iter()
                .map(|(ms, d)| Pos2::new(x(*ms), y(*d)))
                .collect();
            // The feathered mesh fades smoothly, including beyond both endpoints.
            crate::glow::stroke(&p, &points, ICE, design::CURVE_STROKE, design::CURVE_HALO);
        }
        p.text(
            Pos2::new(plot.left() - 20., plot.bottom()),
            Align2::RIGHT_CENTER,
            format!("{min:.0} ms"),
            FontId::proportional(11.),
            MUTED,
        );
        p.text(
            Pos2::new(plot.right() + 20., plot.bottom()),
            Align2::LEFT_CENTER,
            format!("{max:.0} ms"),
            FontId::proportional(11.),
            MUTED,
        );
        p.text(
            Pos2::new(rect.center().x, rect.bottom() - 8.),
            Align2::CENTER_CENTER,
            format!(
                "{:.0} ms average · {:.0} ms spread",
                self.model.mean_ms, self.model.effective_deviation_ms
            ),
            FontId::proportional(11.),
            MUTED,
        );
        let now = Instant::now();
        while self
            .pulses
            .front()
            .is_some_and(|pulse| now.duration_since(pulse.born).as_secs_f32() > 1.15)
        {
            self.pulses.pop_front();
        }
        for pulse in &self.pulses {
            let age = now.duration_since(pulse.born).as_secs_f32();
            let alpha = (1. - age / 1.15).max(0.).powf(1.5);
            let rise = (age / 0.2).clamp(0., 1.);
            let eased = 1. - (1. - rise).powi(3);
            let xx = x(pulse.ms);
            let tip = Pos2::new(
                xx,
                plot.bottom() + (y(density_at(pulse.ms)) - plot.bottom()) * eased,
            );
            let color = Color32::from_rgba_unmultiplied(230, 246, 255, (alpha * 235.) as u8);
            crate::glow::stroke(&p, &[Pos2::new(xx, plot.bottom()), tip], color, 1.4, 2.5);
            crate::glow::stroke(&p, &[tip], color, 3., 3.);
            if age > 0.2 {
                p.circle_stroke(
                    Pos2::new(xx, y(density_at(pulse.ms))),
                    4. + (age - 0.2) * 13.,
                    Stroke::new(
                        1.,
                        Color32::from_rgba_unmultiplied(225, 244, 254, (alpha * 55.) as u8),
                    ),
                );
            }
        }
        if !self.pulses.is_empty() {
            ui.ctx().request_repaint_after(Duration::from_millis(16));
        }
    }
    fn footer(&mut self, ui: &mut egui::Ui) {
        centered_group(ui, |ui| {
            ui.spacing_mut().item_spacing = Vec2::new(8., 5.);
            ui.separator();
            ui.add_space(4.);
            self.metrics(ui);
            ui.add_space(4.);
            ui.horizontal(|ui| {
                fixed_label(
                    ui,
                    "Target duration",
                    112.,
                    design::FIELD_HEIGHT,
                    FontId::proportional(12.),
                    MUTED,
                );
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.target_minutes)
                        .font(FontId::proportional(13.))
                        .desired_width(52.)
                        .frame(entry_frame())
                        .hint_text("—"),
                );
                ui.label(RichText::new("min").size(12.).color(MUTED));
            });
            let (feedback, color) = if self.settings.target_minutes.is_empty() {
                (String::new(), MUTED)
            } else if let Ok(minutes) = self.settings.target_minutes.parse::<f64>()
                && let Some(wpm) = required_wpm(self.prepared.len(), minutes)
            {
                let feasible = target_feasible(&self.last_valid, self.prepared.len(), minutes);
                (
                    format!(
                        "{wpm:.1} WPM required · {}",
                        if feasible {
                            "within delay bounds"
                        } else {
                            "outside delay bounds"
                        }
                    ),
                    if feasible { MUTED } else { ERROR },
                )
            } else {
                ("Enter a positive duration in minutes.".into(), ERROR)
            };
            let (feedback_rect, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 16.), egui::Sense::hover());
            ui.painter().text(
                feedback_rect.left_center(),
                Align2::LEFT_CENTER,
                feedback,
                FontId::proportional(11.),
                color,
            );
            ui.horizontal(|ui| {
                let current = usize::from(self.settings.mode == ActivationMode::Hotkey);
                ui.add_enabled_ui(!self.status.state.active(), |ui| {
                    if let Some(index) = segmented(ui, &["Cursor", "Hotkey"], current, 168.) {
                        self.settings.mode = if index == 0 {
                            ActivationMode::Cursor
                        } else {
                            ActivationMode::Hotkey
                        };
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let summary = if self.settings.mode == ActivationMode::Cursor {
                        format!("{}s countdown", self.settings.startup_seconds)
                    } else {
                        self.settings.hotkey.label()
                    };
                    ui.label(RichText::new(summary).size(11.).color(MUTED));
                });
            });
            ui.add_space(3.);
            let (label, enabled) = match self.status.state {
                SessionState::Typing => ("Pause".into(), true),
                SessionState::Paused => ("Resume after countdown".into(), true),
                SessionState::Countdown => (
                    format!("Starting in {:.1}s · Pause", self.status.countdown),
                    true,
                ),
                SessionState::WaitingForKeys => ("Release keys · Pause".into(), true),
                _ if self.settings.mode == ActivationMode::Hotkey => {
                    (self.settings.hotkey.label(), false)
                }
                _ => (
                    format!("Start in {}s", self.settings.startup_seconds),
                    !self.settings.text.is_empty() && self.timing_error.is_none(),
                ),
            };
            ui.horizontal(|ui| {
                let width = (ui.available_width() - design::ICON_HIT - 8.).max(160.);
                let clicked = ui
                    .add_enabled_ui(enabled, |ui| {
                        text_button(ui, &label, Vec2::new(width, 38.), false, true).clicked()
                    })
                    .inner;
                if clicked {
                    self.toggle(false);
                }
                if icon_button(ui, Icon::Stop, "Stop and reset", false) {
                    self.stop();
                }
            });
            if self.status.total > 0 {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "{} / {} characters",
                            self.status.position, self.status.total
                        ))
                        .size(11.)
                        .color(MUTED),
                    );
                    if self.status.state.active() {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new("Esc to stop").size(11.).color(MUTED));
                        });
                    }
                });
            }
            let message = if !self.status.message.is_empty() {
                Some(&self.status.message)
            } else {
                self.hotkey_error.as_ref()
            };
            if let Some(message) = message {
                ui.label(RichText::new(message).size(11.).color(
                    if self.status.state == SessionState::Error {
                        ERROR
                    } else {
                        MUTED
                    },
                ));
            }
        });
    }
    fn pill(&mut self, ui: &mut egui::Ui) {
        self.header(ui);
        if self.status.total > 0 {
            let rect = ui.max_rect();
            let fraction = self.status.position as f32 / self.status.total as f32;
            ui.painter().line_segment(
                [
                    Pos2::new(rect.left() + 30., rect.bottom() - 2.),
                    Pos2::new(
                        rect.left() + 30. + (rect.width() - 190.) * fraction,
                        rect.bottom() - 2.,
                    ),
                ],
                Stroke::new(2., ICE),
            );
        }
    }
    fn settings_panel(&mut self, ui: &mut egui::Ui, emphasis: f32) {
        centered_group(ui, |ui| {
            ui.horizontal(|ui| {
                fixed_label(
                    ui,
                    "Settings",
                    110.,
                    36.,
                    design::settings_title_font(),
                    INK,
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if quiet_button(ui, "← Back to typing").clicked() {
                        self.controls_open = false;
                        ui.ctx().request_repaint();
                    }
                });
            });
            ui.add_space(5.);
            let height = (ui.available_height() - 30.).max(32.);
            egui::Frame::new()
                .fill(Color32::from_rgba_unmultiplied(
                    design::SURFACE[0],
                    design::SURFACE[1],
                    design::SURFACE[2],
                    (emphasis * f32::from(design::SETTINGS_TINT)) as u8,
                ))
                .stroke(Stroke::new(
                    1.,
                    Color32::from_white_alpha((emphasis * 22.) as u8),
                ))
                .corner_radius(design::SETTINGS_RADIUS)
                .inner_margin(14)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("controls-scroll")
                        .auto_shrink([false, false])
                        .max_height(height)
                        .min_scrolled_height(height)
                        .show(ui, |ui| self.controls(ui));
                });
        });
    }
    fn controls(&mut self, ui: &mut egui::Ui) {
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(8., 8.);
            ui.label(RichText::new("Appearance").font(design::heading_font()));
            ui.add(
                egui::Slider::new(&mut self.settings.glass_tint, 40..=210)
                    .custom_formatter(|v, _| format!("{:.0}%", v / 255. * 100.))
                    .text("Dark tint"),
            );
            if ui
                .checkbox(&mut self.settings.blur, "Blur background")
                .changed()
                && let Some(hwnd) = self.native_hwnd
            {
                platform::apply_glass(hwnd, self.settings.blur);
            }
            ui.label(RichText::new("Softens the desktop behind this window, including while typing in another app.").size(11.).color(MUTED));
            if self.settings.blur
                && let Some(error) = self.native_hwnd
                    .map(platform::blur_status)
                    .unwrap_or_else(|| Err("Background blur is unavailable.".into()))
                    .err()
            {
                ui.label(
                    RichText::new(error)
                        .size(11.)
                        .color(ERROR),
                );
            }
            ui.add_space(6.);
            ui.separator();
            ui.add_space(4.);
            ui.label(RichText::new("Activation").font(design::heading_font()));
            ui.add_enabled_ui(!self.status.state.active(), |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Startup delay").size(12.));
                    number_field(ui, &mut self.settings.startup_seconds, 0. ..=60., " s", 80.);
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Line breaks").size(12.));
                    let selected = usize::from(self.settings.newline == NewlineMode::ShiftEnter);
                    if let Some(index) = segmented(ui, &["Enter", "Shift + Enter"], selected, 184.)
                    {
                        self.settings.newline = if index == 0 {
                            NewlineMode::Enter
                        } else {
                            NewlineMode::ShiftEnter
                        };
                    }
                });
            });
            ui.add_space(6.);
            ui.separator();
            ui.add_space(4.);
            ui.label(RichText::new("Typing hotkey").font(design::heading_font()));
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.hotkey_draft.control, "Ctrl");
                ui.checkbox(&mut self.hotkey_draft.alt, "Alt");
                ui.checkbox(&mut self.hotkey_draft.shift, "Shift");
                ui.checkbox(&mut self.hotkey_draft.windows, "Win");
            });
            ui.horizontal(|ui| {
                ui.add(
                    egui::DragValue::new(&mut self.hotkey_draft.function)
                        .range(1..=24)
                        .prefix("F"),
                );
                if quiet_button(ui, "Apply hotkey").clicked()
                    && let Some(h) = &self.hotkeys
                {
                    let _ =
                        h.tx.send(HotkeyCommand::Configure(self.hotkey_draft.clone()));
                }
            });
            if let Some(error) = &self.hotkey_error {
                ui.label(RichText::new(error).color(ERROR).size(11.));
            }
            ui.add_space(4.);
            ui.label(
                RichText::new(
                    "Focus protection tracks windows, not browser tabs or caret changes.",
                )
                .size(11.)
                .color(MUTED),
            );
            if quiet_button(ui, "Forget saved text").clicked() && !self.status.state.active() {
                self.settings.text.clear();
                self.dirty_at = Some(Instant::now());
                self.persist();
            }
        });
    }
}

impl eframe::App for ShardApp {
    fn logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.drain();
        if let Some(at) = self.dirty_at {
            if at.elapsed() > Duration::from_millis(600) {
                self.persist();
            } else {
                ctx.request_repaint_after(Duration::from_millis(650));
            }
        }
        if let Some(path) = &self.preview.screenshot {
            let screenshot = ctx.input(|i| {
                i.events.iter().find_map(|e| {
                    if let egui::Event::Screenshot { image, .. } = e {
                        Some(image.clone())
                    } else {
                        None
                    }
                })
            });
            if let Some(image) = screenshot {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let pixels: Vec<u8> = image
                    .pixels
                    .iter()
                    .flat_map(|c| c.to_srgba_unmultiplied())
                    .collect();
                if let Err(e) = image::save_buffer(
                    path,
                    &pixels,
                    image.size[0] as u32,
                    image.size[1] as u32,
                    image::ColorType::Rgba8,
                ) {
                    eprintln!("Screenshot failed: {e}");
                }
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
        }
    }
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let before = self.settings.clone();
        let ctx = ui.ctx().clone();
        let rect = ui.max_rect();
        if let Some(window) = frame.winit_window() {
            let size = window.inner_size();
            let scale = ctx.pixels_per_point();
            let shape = [size.width, size.height, (scale * 1000.).round() as u32];
            if self.window_shape != Some(shape)
                && let Some(hwnd) = self.native_hwnd
            {
                platform::shape_window(hwnd, [size.width, size.height], design::CORNER * scale);
                self.window_shape = Some(shape);
            }
        }
        let background = Color32::from_rgba_unmultiplied(
            design::SURFACE[0],
            design::SURFACE[1],
            design::SURFACE[2],
            self.settings.glass_tint,
        );
        let points =
            shard_typer::silhouette::continuous_rect([rect.width(), rect.height()], design::CORNER)
                .into_iter()
                .map(|p| rect.min + Vec2::new(p[0], p[1]))
                .collect();
        ui.painter().add(egui::Shape::convex_polygon(
            points,
            background,
            Stroke::NONE,
        ));
        let inset = rect.shrink(0.75);
        let border = shard_typer::silhouette::continuous_rect(
            [inset.width(), inset.height()],
            design::CORNER - 0.75,
        )
        .into_iter()
        .map(|p| inset.min + Vec2::new(p[0], p[1]))
        .collect();
        ui.painter().add(egui::Shape::closed_line(
            border,
            Stroke::new(1., design::BORDER),
        ));
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(rect.shrink2(design::PADDING)),
            |ui| {
                if self.settings.collapsed {
                    self.pill(ui);
                } else {
                    self.header(ui);
                    let settings_emphasis = ctx.animate_bool_with_time(
                        egui::Id::new("settings-panel"),
                        self.controls_open,
                        design::SETTINGS_TRANSITION,
                    );
                    egui::Panel::bottom("run-controls")
                        .show_separator_line(false)
                        .frame(egui::Frame::NONE)
                        .show(ui, |ui| self.footer(ui));
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE)
                        .show(ui, |ui| {
                            if self.controls_open {
                                self.settings_panel(ui, settings_emphasis);
                            } else {
                                self.split_panels(ui);
                            }
                        });
                }
            },
        );
        if let Some(notice) = self.notice.clone() {
            egui::Window::new("A quick note")
                .collapsible(false)
                .resizable(false)
                .default_width(340.)
                .show(&ctx, |ui| {
                    ui.label(&notice);
                    if quiet_button(ui, "Got it").clicked() {
                        self.notice = None;
                    }
                });
        }
        if self.settings.timing != self.last_valid {
            match self.settings.timing.validate() {
                Ok(()) => {
                    self.last_valid = self.settings.timing.clone();
                    self.model = self.last_valid.model();
                    self.timing_error = None;
                    self.send(Command::Timing(self.last_valid.clone()));
                    ctx.request_repaint();
                }
                Err(e) => self.timing_error = Some(e),
            }
        } else {
            self.timing_error = None;
        }
        ctx.input(|i| {
            if !self.settings.collapsed
                && let Some(inner) = i.viewport().inner_rect
            {
                self.settings.expanded_size = [inner.width().max(400.), inner.height().max(640.)];
            }
        });
        if let Some(position) = frame
            .winit_window()
            .and_then(|window| window.outer_position().ok())
        {
            self.settings.position = Some([position.x as f32, position.y as f32]);
        }
        if self.settings != before {
            self.dirty_at = Some(Instant::now());
            ctx.request_repaint_after(Duration::from_millis(650));
        }
        self.frames += 1;
        if self.preview.screenshot.is_some()
            && self.frames >= 4
            && self.created_at.elapsed() >= Duration::from_millis(350)
            && !self.screenshot_requested
        {
            self.screenshot_requested = true;
            ctx.send_viewport_cmd(ViewportCommand::Screenshot(egui::UserData::default()));
        } else if self.preview.screenshot.is_some() && !self.screenshot_requested {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::TRANSPARENT.to_array()
    }
}
impl Drop for ShardApp {
    fn drop(&mut self) {
        self.send(Command::Stop);
        self.persist();
    }
}

fn style(ctx: &egui::Context) {
    let mut style = egui::Style {
        visuals: egui::Visuals::dark(),
        ..Default::default()
    };
    style.visuals.override_text_color = Some(INK);
    // Disable interaction without fading foreground text with its parent UI.
    style.visuals.disabled_alpha = 1.;
    style.visuals.panel_fill = Color32::TRANSPARENT;
    style.visuals.window_fill = Color32::from_rgb(26, 31, 39);
    style.visuals.window_stroke = Stroke::new(1., Color32::from_white_alpha(38));
    style.visuals.window_corner_radius = 12.into();
    style.visuals.widgets.inactive.bg_fill = Color32::from_white_alpha(12);
    style.visuals.widgets.inactive.weak_bg_fill = Color32::from_white_alpha(12);
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1., Color32::from_white_alpha(18));
    style.visuals.widgets.hovered.bg_fill = Color32::from_white_alpha(24);
    style.visuals.widgets.active.bg_fill = Color32::from_white_alpha(36);
    style.visuals.widgets.noninteractive.corner_radius = 9.into();
    style.visuals.widgets.inactive.corner_radius = 9.into();
    style.visuals.widgets.hovered.corner_radius = 9.into();
    style.visuals.widgets.active.corner_radius = 9.into();
    style.visuals.widgets.open.corner_radius = 9.into();
    style.visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(202, 227, 244, 52);
    style.spacing.item_spacing = Vec2::new(6., 6.);
    style.spacing.button_padding = Vec2::new(10., 5.);
    style.spacing.slider_width = 170.;
    style.spacing.scroll.bar_width = 4.;
    style.spacing.scroll.floating_width = 4.;
    style.spacing.scroll.floating_allocated_width = 6.;
    style.spacing.scroll.dormant_handle_opacity = 0.45;
    style.spacing.scroll.dormant_background_opacity = 0.;
    style
        .text_styles
        .insert(egui::TextStyle::Body, FontId::proportional(13.));
    style
        .text_styles
        .insert(egui::TextStyle::Button, FontId::proportional(12.));
    ctx.set_style_of(egui::Theme::Dark, style);
    ctx.set_theme(egui::Theme::Dark);
    let mut fonts = egui::FontDefinitions::default();
    for (name, path) in [
        ("segoe", "C:\\Windows\\Fonts\\segoeui.ttf"),
        ("segoe-semibold", "C:\\Windows\\Fonts\\seguisb.ttf"),
        ("symbols", "C:\\Windows\\Fonts\\seguisym.ttf"),
        ("emoji", "C:\\Windows\\Fonts\\seguiemj.ttf"),
    ] {
        if let Ok(bytes) = std::fs::read(path) {
            fonts
                .font_data
                .insert(name.into(), Arc::new(egui::FontData::from_owned(bytes)));
            if name != "segoe-semibold" {
                fonts
                    .families
                    .entry(egui::FontFamily::Proportional)
                    .or_default()
                    .push(name.into());
            }
        }
    }
    if fonts.font_data.contains_key("segoe") {
        let family = fonts
            .families
            .get_mut(&egui::FontFamily::Proportional)
            .unwrap();
        family.retain(|name| name != "segoe");
        family.insert(0, "segoe".into());
    }
    let mut headings = fonts.families[&egui::FontFamily::Proportional].clone();
    if fonts.font_data.contains_key("segoe-semibold") {
        headings.insert(0, "segoe-semibold".into());
    }
    fonts
        .families
        .insert(egui::FontFamily::Name("shard-heading".into()), headings);
    ctx.set_fonts(fonts);
}
fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(Color32::from_black_alpha(24))
        .stroke(Stroke::new(1., Color32::from_white_alpha(18)))
        .corner_radius(12)
}
fn entry_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(design::FIELD)
        .corner_radius(design::FIELD_RADIUS)
        .inner_margin(egui::Margin::symmetric(9, 6))
}
fn centered_at<R>(ui: &mut egui::Ui, width: f32, content: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let width = width.min(ui.available_width());
    let height = ui.available_height();
    let offset = ((ui.available_width() - width) / 2.).max(0.);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.;
        ui.add_space(offset);
        ui.vertical(|ui| {
            ui.set_width(width);
            ui.set_max_width(width);
            // Horizontal rows start with a one-line height. Restore the
            // parent's vertical budget for editors and scrolling settings.
            ui.set_max_height(height);
            ui.spacing_mut().item_spacing = Vec2::new(8., 6.);
            content(ui)
        })
        .inner
    })
    .inner
}
fn centered_group<R>(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui) -> R) -> R {
    centered_at(ui, design::GROUP_WIDTH, content)
}
fn interaction_fill(ui: &egui::Ui, response: &egui::Response, rect: Rect, selected: bool) {
    let hover = ui.ctx().animate_bool_with_time(
        response.id.with("hover"),
        response.hovered(),
        design::HOVER_SECONDS,
    );
    let alpha = if response.is_pointer_button_down_on() {
        design::PRESSED
    } else if selected {
        design::SELECTED + (hover * 7.) as u8
    } else {
        (hover * f32::from(design::HOVER)) as u8
    };
    if alpha > 0 {
        ui.painter()
            .rect_filled(rect, design::FIELD_RADIUS, Color32::from_white_alpha(alpha));
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(0.5),
            design::FIELD_RADIUS,
            Stroke::new(1., ICE),
            egui::StrokeKind::Inside,
        );
    }
}
fn text_button(
    ui: &mut egui::Ui,
    label: &str,
    size: Vec2,
    selected: bool,
    primary: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    if primary {
        ui.painter()
            .rect_filled(rect, 11, if ui.is_enabled() { ICE } else { design::FIELD });
    }
    interaction_fill(ui, &response, rect, selected);
    let color = if !ui.is_enabled() {
        design::DISABLED
    } else if primary {
        Color32::from_rgb(20, 34, 43)
    } else if selected {
        INK
    } else {
        MUTED
    };
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(if primary { 14. } else { 12. }),
        color,
    );
    response
}
fn quiet_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let width = ui
        .painter()
        .layout_no_wrap(label.into(), FontId::proportional(12.), MUTED)
        .size()
        .x
        + 16.;
    text_button(ui, label, Vec2::new(width, 30.), false, false)
}
fn fixed_label(
    ui: &mut egui::Ui,
    label: &str,
    width: f32,
    height: f32,
    font: FontId,
    color: Color32,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::hover());
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), label));
    ui.painter()
        .text(rect.left_center(), Align2::LEFT_CENTER, label, font, color);
    response
}
fn segmented(ui: &mut egui::Ui, labels: &[&str], selected: usize, width: f32) -> Option<usize> {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 30.), egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, 10, Color32::from_white_alpha(7));
    let segment = (width - 6.) / labels.len() as f32;
    let mut result = None;
    for (index, label) in labels.iter().enumerate() {
        let cell = Rect::from_min_size(
            rect.min + Vec2::new(3. + index as f32 * segment, 3.),
            Vec2::new(segment, 24.),
        );
        let response = ui.interact(
            cell,
            ui.id().with(("segment", labels[0], index)),
            egui::Sense::click(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::RadioButton,
                ui.is_enabled(),
                index == selected,
                *label,
            )
        });
        interaction_fill(ui, &response, cell, index == selected);
        ui.painter().text(
            cell.center(),
            Align2::CENTER_CENTER,
            *label,
            FontId::proportional(12.),
            if !ui.is_enabled() {
                design::DISABLED
            } else if index == selected {
                INK
            } else {
                MUTED
            },
        );
        if response.clicked() {
            result = Some(index);
        }
    }
    result
}
fn metric(ui: &mut egui::Ui, label: &str, value: &str, unit: &str) {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 42.), egui::Sense::hover());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Label,
            ui.is_enabled(),
            format!("{label}: {value} {unit}"),
        )
    });
    ui.painter().text(
        Pos2::new(rect.center().x, rect.top() + 7.),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(11.),
        MUTED,
    );
    let gap = if unit.is_empty() { 0. } else { 5. };
    let value = ui
        .painter()
        .layout_no_wrap(value.into(), FontId::proportional(20.), INK);
    let unit = ui
        .painter()
        .layout_no_wrap(unit.into(), FontId::proportional(11.), MUTED);
    let left = rect.center().x - (value.size().x + gap + unit.size().x) / 2.;
    let baseline = rect.bottom() - 2.;
    ui.painter().galley(
        Pos2::new(left, baseline - value.size().y),
        value.clone(),
        INK,
    );
    ui.painter().galley(
        Pos2::new(left + value.size().x + gap, baseline - unit.size().y - 2.),
        unit,
        MUTED,
    );
}
fn number_field(
    ui: &mut egui::Ui,
    value: &mut f64,
    range: std::ops::RangeInclusive<f64>,
    suffix: &str,
    width: f32,
) -> egui::Response {
    let (rect, field_response) =
        ui.allocate_exact_size(Vec2::new(width, design::FIELD_HEIGHT), egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, design::FIELD_RADIUS, design::FIELD);
    ui.painter()
        .rect_stroke(rect, 9, Stroke::NONE, egui::StrokeKind::Inside);
    let hover = ui.ctx().animate_bool_with_time(
        field_response.id.with("field-hover"),
        ui.rect_contains_pointer(rect),
        design::HOVER_SECONDS,
    );
    if hover > 0. {
        ui.painter().rect_filled(
            rect,
            design::FIELD_RADIUS,
            Color32::from_white_alpha((hover * f32::from(design::HOVER)) as u8),
        );
    }
    let inner = ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(7., 4.)))
            .layout(egui::Layout::left_to_right(egui::Align::Center).with_main_justify(true)),
        |ui| {
            let visuals = &mut ui.style_mut().visuals;
            for state in [
                &mut visuals.widgets.inactive,
                &mut visuals.widgets.hovered,
                &mut visuals.widgets.active,
            ] {
                state.bg_fill = Color32::TRANSPARENT;
                state.weak_bg_fill = Color32::TRANSPARENT;
                state.bg_stroke = Stroke::NONE;
            }
            ui.spacing_mut().button_padding = Vec2::ZERO;
            ui.spacing_mut().interact_size = Vec2::new(width - 14., 20.);
            ui.add(
                egui::DragValue::new(value)
                    .speed(1.)
                    .range(range)
                    .suffix(suffix)
                    .max_decimals(1),
            )
        },
    );
    let response = inner.inner;
    if response.has_focus() || response.hovered() {
        ui.painter().rect_stroke(
            rect,
            9,
            Stroke::new(
                1.,
                Color32::from_white_alpha(if response.has_focus() { 95 } else { 40 }),
            ),
            egui::StrokeKind::Inside,
        );
    }
    response
}
fn slider_row(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f64,
    range: std::ops::RangeInclusive<f64>,
    suffix: &str,
    help: &str,
    midpoint: Option<f64>,
) {
    ui.horizontal(|ui| {
        let slider_width = (ui.available_width() - 76. - design::FIELD_WIDTH - 36. - 24.).max(60.);
        fixed_label(
            ui,
            label,
            76.,
            design::FIELD_HEIGHT,
            FontId::proportional(12.),
            MUTED,
        )
        .on_hover_ui(|ui| {
            ui.set_max_width(260.);
            ui.label(help);
        });
        ui.scope(|ui| {
            ui.spacing_mut().slider_width = slider_width;
            ui.add(
                egui::Slider::new(value, range.clone())
                    .show_value(false)
                    .handle_shape(egui::style::HandleShape::Circle)
                    .trailing_fill(false),
            )
            .on_hover_ui(|ui| {
                ui.set_max_width(260.);
                ui.label(help);
            });
        });
        number_field(ui, value, range, suffix, design::FIELD_WIDTH);
        if let Some(midpoint) = midpoint {
            if text_button(
                ui,
                "Mid",
                Vec2::new(36., design::FIELD_HEIGHT),
                false,
                false,
            )
            .on_hover_text("Set Center halfway between Min and Max")
            .clicked()
            {
                *value = midpoint;
            }
        } else {
            ui.allocate_exact_size(Vec2::new(36., design::FIELD_HEIGHT), egui::Sense::hover());
        }
    });
}
fn duration(seconds: f64) -> String {
    let total = seconds.round() as u64;
    let (m, s) = (total / 60, total % 60);
    if m >= 60 {
        format!("{}h {}m", m / 60, m % 60)
    } else {
        format!("{m}:{s:02}")
    }
}
fn shard(p: &egui::Painter, r: Rect, color: Color32) {
    let a = r.min + Vec2::new(r.width() * 0.62, 1.);
    let b = r.min + Vec2::new(2., r.height() * 0.64);
    let c = r.max - Vec2::new(r.width() * 0.35, 1.);
    let d = r.min + Vec2::new(r.width() - 1., r.height() * 0.33);
    p.add(egui::Shape::convex_polygon(
        vec![a, b, c],
        color,
        Stroke::NONE,
    ));
    p.add(egui::Shape::convex_polygon(
        vec![a, c, d],
        Color32::from_rgb(168, 188, 197),
        Stroke::NONE,
    ));
    p.line_segment([a, c], Stroke::new(1., Color32::from_white_alpha(130)));
}
fn icon_button(ui: &mut egui::Ui, icon: Icon, label: &str, selected: bool) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::splat(design::ICON_HIT), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    let settings_icon = matches!(icon, Icon::Settings);
    interaction_fill(ui, &response, rect, selected && !settings_icon);
    if settings_icon {
        let emphasis = ui.ctx().animate_bool_with_time(
            response.id.with("settings-selected"),
            selected,
            design::SETTINGS_TRANSITION,
        );
        if emphasis > 0. {
            ui.painter().rect_filled(
                rect,
                design::FIELD_RADIUS,
                Color32::from_white_alpha((emphasis * f32::from(design::SETTINGS_SELECTED)) as u8),
            );
            ui.painter().rect_stroke(
                rect.shrink(0.5),
                design::FIELD_RADIUS,
                Stroke::new(1., Color32::from_white_alpha((emphasis * 55.) as u8)),
                egui::StrokeKind::Inside,
            );
        }
    }
    let p = ui.painter();
    let c = rect.center();
    let s = Stroke::new(design::ICON_STROKE, if selected { ICE } else { MUTED });
    let pt = |x: f32, y: f32| c + Vec2::new(x, y);
    let line = |a: [f32; 2], b: [f32; 2]| {
        p.line_segment([pt(a[0], a[1]), pt(b[0], b[1])], s);
    };
    match icon {
        Icon::Close => {
            line([-6., -6.], [6., 6.]);
            line([-6., 6.], [6., -6.]);
        }
        Icon::Minimize => {
            line([-7., 2.], [7., 2.]);
        }
        Icon::Play => {
            p.add(egui::Shape::convex_polygon(
                vec![pt(-4., -6.), pt(-4., 6.), pt(6., 0.)],
                s.color,
                Stroke::NONE,
            ));
        }
        Icon::Pause => {
            line([-3., -6.], [-3., 6.]);
            line([3., -6.], [3., 6.]);
        }
        Icon::Stop => {
            p.rect_stroke(
                Rect::from_center_size(c, Vec2::splat(12.)),
                2,
                s,
                egui::StrokeKind::Inside,
            );
        }
        Icon::Settings => {
            for (y, x) in [(-5., 3.), (0., -3.), (5., 2.)] {
                line([-7., y], [x - 2., y]);
                line([x + 2., y], [7., y]);
                p.circle_stroke(pt(x, y), 2., s);
            }
        }
        Icon::Pin => {
            line([-4., -7.], [4., -7.]);
            p.add(egui::Shape::line(
                vec![
                    pt(-3., -7.),
                    pt(-3., -1.),
                    pt(-6., 2.),
                    pt(6., 2.),
                    pt(3., -1.),
                    pt(3., -7.),
                ],
                s,
            ));
            line([0., 2.], [0., 8.]);
        }
        Icon::Collapse | Icon::Expand => {
            p.rect_stroke(
                Rect::from_center_size(c, Vec2::new(17., 14.)),
                3,
                s,
                egui::StrokeKind::Inside,
            );
            line([-7., 4.], [7., 4.]);
            let direction = if matches!(icon, Icon::Collapse) {
                1.
            } else {
                -1.
            };
            line([-3., -2. * direction], [0., 1. * direction]);
            line([0., 1. * direction], [3., -2. * direction]);
        }
    }
    response.on_hover_text(label).clicked()
}
