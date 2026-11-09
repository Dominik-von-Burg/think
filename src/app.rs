//! The notebook's state and the frame it paints.

use std::{
    fs,
    path::Path,
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
    time::{Duration, Instant},
};

use eframe::egui::{self, Color32, FontId, TextEdit};

use serde::{Deserialize, Serialize};

use crate::markdown::{link_at_char, markdown_layouter};
use crate::paths::Paths;
use crate::scroll::*;
use crate::sync::commit_and_push;
use crate::title_bar::*;
use crate::trace::scroll_log;
use crate::{configure_context, set_windows_title_bar_colors, APP_NAME};

#[cfg(target_os = "windows")]
use crate::win_chrome;

pub(crate) const DEBOUNCE: Duration = Duration::from_secs(3);

pub(crate) const SAVE_DEBOUNCE: Duration = Duration::from_millis(400);

#[derive(Default, Serialize, Deserialize, Clone)]
pub(crate) struct Settings {
    repo_url: String,
}

pub(crate) struct NotebookApp {
    text: String,
    paths: Paths,
    settings: Settings,
    /// Token field in the settings dialog. The real token lives in the OS keyring.
    token_input: String,
    show_settings: bool,
    settings_error: Option<String>,
    dirty_since: Option<Instant>,
    last_change: Option<Instant>,
    sync_in_progress: bool,
    sync_error: Option<String>,
    sync_receiver: Option<Receiver<Result<(), String>>>,
    /// Painted strength of each fade band, 0.0..=1.0, eased towards the strength
    /// the scroll position calls for so a band never pops in or out.
    fade_top: f32,
    fade_bottom: f32,
    /// The last pass moved the view and asked egui to paint the frame again: trace
    /// the pass that replaces it, which is the one on screen.
    trace_repaint: bool,
}

impl NotebookApp {
    pub(crate) fn new(cc: &eframe::CreationContext<'_>) -> Self {
        configure_context(&cc.egui_ctx);
        set_windows_title_bar_colors(cc);
        #[cfg(target_os = "windows")]
        win_chrome::install_from(cc);
        let paths = Paths::new();
        let settings = load_settings(&paths.settings);
        let text = fs::read_to_string(&paths.notebook).unwrap_or_default();
        Self::with(paths, settings, text)
    }

    fn with(paths: Paths, settings: Settings, text: String) -> Self {
        let token_input = crate::keyring_store::load_token().unwrap_or_default();
        Self {
            text,
            paths,
            settings,
            token_input,
            show_settings: false,
            settings_error: None,
            dirty_since: None,
            last_change: None,
            sync_in_progress: false,
            sync_error: None,
            sync_receiver: None,
            fade_top: 0.0,
            fade_bottom: 0.0,
            trace_repaint: false,
        }
    }

    fn save_notebook(&mut self) -> Result<(), String> {
        fs::create_dir_all(&self.paths.data_dir).map_err(|e| e.to_string())?;
        fs::write(&self.paths.notebook, &self.text).map_err(|e| e.to_string())
    }

    fn save_settings(&self) -> Result<(), String> {
        fs::create_dir_all(&self.paths.data_dir).map_err(|e| e.to_string())?;
        let contents = toml::to_string(&self.settings).map_err(|e| e.to_string())?;
        fs::write(&self.paths.settings, contents).map_err(|e| e.to_string())?;
        if self.token_input.trim().is_empty() {
            crate::keyring_store::delete_token().ok();
        } else {
            crate::keyring_store::save_token(self.token_input.trim())?;
        }
        Ok(())
    }

    fn start_sync(&mut self) {
        let token = match crate::keyring_store::load_token() {
            Ok(t) if !t.trim().is_empty() => t,
            _ => return,
        };
        if self.settings.repo_url.trim().is_empty() || self.sync_in_progress {
            return;
        }

        let notebook = self.paths.notebook.clone();
        let repository = self.paths.repository.clone();
        let repo_url = self.settings.repo_url.clone();
        let (sender, receiver) = mpsc::channel();
        self.sync_in_progress = true;
        self.sync_error = None;
        self.sync_receiver = Some(receiver);

        thread::spawn(move || {
            let _ = sender.send(commit_and_push(&notebook, &repository, &repo_url, &token));
        });
    }

    fn collect_finished_sync(&mut self) {
        let result = match self.sync_receiver.as_ref() {
            Some(receiver) => match receiver.try_recv() {
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => Err("sync worker stopped".to_owned()),
                Ok(result) => result,
            },
            None => return,
        };
        self.sync_in_progress = false;
        self.sync_receiver = None;
        if let Err(message) = result {
            self.sync_error = Some(message);
        }
    }

    /// Always-visible sync status in the title bar. Covers every state so a
    /// sync that can't happen (not configured) or did fail is never silent.
    fn sync_status(&self, ui: &mut egui::Ui) {
        let configured =
            !self.settings.repo_url.trim().is_empty() && crate::keyring_store::has_token();
        if self.sync_in_progress {
            sync_indicator(ui).on_hover_text("Syncing to GitHub…");
        } else if let Some(error) = &self.sync_error {
            ui.label(
                egui::RichText::new("Sync failed")
                    .color(Color32::from_rgb(232, 120, 120))
                    .size(12.0),
            )
            .on_hover_text(error.as_str());
        } else if !configured {
            ui.label(egui::RichText::new("Not syncing").weak().size(12.0))
                .on_hover_text("Add a GitHub repository and token in settings to sync your notes.");
        } else if self.last_change.is_some() {
            sync_indicator(ui).on_hover_text("Waiting to sync…");
        }
    }

    fn show_title_bar(&mut self, ctx: &egui::Context) {
        // Custom caption with native-style window buttons: 46px wide, flush
        // with the top-right corner, subtle hover fill (red for close) and
        // thin 1px glyphs. App controls live left of the buttons.
        const CAPTION_BUTTON_WIDTH: f32 = 46.0;
        let screen = ctx.screen_rect();
        egui::Area::new(egui::Id::new("title_bar"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.left_top())
            .show(ctx, |ui| {
                let title_bar_rect = egui::Rect::from_min_size(
                    ui.min_rect().min,
                    egui::vec2(screen.width(), TITLE_BAR_HEIGHT),
                );
                // No opaque fill: scrolled text fades out underneath the bar
                // via the top scroll fade instead.

                let maximized = ctx.input(|input| input.viewport().maximized.unwrap_or(false));
                let close_rect = egui::Rect::from_min_size(
                    egui::pos2(
                        title_bar_rect.right() - CAPTION_BUTTON_WIDTH,
                        title_bar_rect.top(),
                    ),
                    egui::vec2(CAPTION_BUTTON_WIDTH, TITLE_BAR_HEIGHT),
                );
                let max_rect = egui::Rect::from_min_size(
                    egui::pos2(
                        title_bar_rect.right() - CAPTION_BUTTON_WIDTH * 2.0,
                        title_bar_rect.top(),
                    ),
                    egui::vec2(CAPTION_BUTTON_WIDTH, TITLE_BAR_HEIGHT),
                );
                let min_rect = egui::Rect::from_min_size(
                    egui::pos2(
                        title_bar_rect.right() - CAPTION_BUTTON_WIDTH * 3.0,
                        title_bar_rect.top(),
                    ),
                    egui::vec2(CAPTION_BUTTON_WIDTH, TITLE_BAR_HEIGHT),
                );

                let mut control_hovered = false;
                let close =
                    caption_button(ui, close_rect, CaptionButton::Close).on_hover_text("Close");
                control_hovered |= close.hovered();
                if close.clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }

                let maximize = caption_button(
                    ui,
                    max_rect,
                    if maximized {
                        CaptionButton::Restore
                    } else {
                        CaptionButton::Maximize
                    },
                );
                // On Windows the maximize button is hit-tested natively
                // (HTMAXBUTTON) for Snap Layouts: the wndproc handles the
                // click and Windows shows the flyout instead of a tooltip.
                #[cfg(not(target_os = "windows"))]
                let maximize =
                    maximize.on_hover_text(if maximized { "Restore" } else { "Maximize" });
                control_hovered |= maximize.hovered();
                if maximize.clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                }

                let minimize =
                    caption_button(ui, min_rect, CaptionButton::Minimize).on_hover_text("Minimize");
                control_hovered |= minimize.hovered();
                if minimize.clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                }

                let content_rect = egui::Rect::from_min_max(
                    title_bar_rect.min + egui::vec2(12.0 + TITLE_X_NUDGE, 2.0),
                    egui::pos2(min_rect.left() - 4.0, title_bar_rect.bottom() - 2.0),
                );
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(content_rect), |ui| {
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(APP_NAME)
                                .color(Color32::from_rgb(232, 232, 232))
                                .size(14.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let settings = title_bar_icon_button(ui, TitleBarIcon::Settings)
                                .on_hover_text("GitHub settings");
                            control_hovered |= settings.hovered();
                            if settings.clicked() {
                                self.show_settings = true;
                            }

                            self.sync_status(ui);
                        });
                    });
                });

                // Publish hit-test metrics for the Win32 subclass (physical px).
                #[cfg(target_os = "windows")]
                {
                    let ppp = ctx.pixels_per_point();
                    win_chrome::set_hit_metrics(
                        (TITLE_BAR_HEIGHT * ppp).round() as u32,
                        (CAPTION_BUTTON_WIDTH * ppp).round() as u32,
                    );
                }

                let (pointer_in_title_bar, primary_pressed, double_clicked) = ctx.input(|input| {
                    (
                        input
                            .pointer
                            .hover_pos()
                            .is_some_and(|position| title_bar_rect.contains(position)),
                        input.pointer.primary_pressed(),
                        input
                            .pointer
                            .button_double_clicked(egui::PointerButton::Primary),
                    )
                });
                if pointer_in_title_bar && !control_hovered {
                    if double_clicked {
                        let maximized =
                            ctx.input(|input| input.viewport().maximized.unwrap_or(false));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                    } else if primary_pressed {
                        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }
                }
            });
    }

    /// Fallback resize drag for platforms without native HT* hit-testing.
    /// On Windows the wndproc subclass handles resize natively; skip
    /// BeginResize there so the two don't fight.
    fn handle_window_resize(&self, ctx: &egui::Context) {
        if cfg!(target_os = "windows") {
            return;
        }
        const RESIZE_BORDER: f32 = 3.0;
        let direction = ctx.input(|input| {
            if !input.pointer.primary_pressed() || input.viewport().maximized == Some(true) {
                return None;
            }
            let pointer = input.pointer.hover_pos()?;
            let screen = input.screen_rect();
            let north = pointer.y - screen.top() < RESIZE_BORDER;
            let south = screen.bottom() - pointer.y < RESIZE_BORDER;
            let west = pointer.x - screen.left() < RESIZE_BORDER;
            let east = screen.right() - pointer.x < RESIZE_BORDER;
            match (north, south, west, east) {
                (true, _, true, _) => Some(egui::ResizeDirection::NorthWest),
                (true, _, _, true) => Some(egui::ResizeDirection::NorthEast),
                (_, true, true, _) => Some(egui::ResizeDirection::SouthWest),
                (_, true, _, true) => Some(egui::ResizeDirection::SouthEast),
                (true, _, _, _) => Some(egui::ResizeDirection::North),
                (_, true, _, _) => Some(egui::ResizeDirection::South),
                (_, _, true, _) => Some(egui::ResizeDirection::West),
                (_, _, _, true) => Some(egui::ResizeDirection::East),
                _ => None,
            }
        });
        if let Some(direction) = direction {
            ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(direction));
        }
    }

    /// Set the resize cursor on hover. Must run last in `update`, after the
    /// TextEdit, or the I-beam overrides it.
    fn update_resize_cursor(&self, ctx: &egui::Context) {
        const RESIZE_BORDER: f32 = 3.0;
        if ctx.input(|input| input.viewport().maximized.unwrap_or(false)) {
            return;
        }
        let direction = ctx.input(|input| {
            // Mid-drag (e.g. scrollbar), keep whatever cursor the widget set.
            if input.pointer.any_down() {
                return None;
            }
            let pointer = input.pointer.hover_pos()?;
            let screen = input.screen_rect();
            let north = pointer.y - screen.top() < RESIZE_BORDER;
            let south = screen.bottom() - pointer.y < RESIZE_BORDER;
            let west = pointer.x - screen.left() < RESIZE_BORDER;
            let east = screen.right() - pointer.x < RESIZE_BORDER;
            match (north, south, west, east) {
                (true, _, true, _) => Some(egui::ResizeDirection::NorthWest),
                (true, _, _, true) => Some(egui::ResizeDirection::NorthEast),
                (_, true, true, _) => Some(egui::ResizeDirection::SouthWest),
                (_, true, _, true) => Some(egui::ResizeDirection::SouthEast),
                (true, _, _, _) => Some(egui::ResizeDirection::North),
                (_, true, _, _) => Some(egui::ResizeDirection::South),
                (_, _, true, _) => Some(egui::ResizeDirection::West),
                (_, _, _, true) => Some(egui::ResizeDirection::East),
                _ => None,
            }
        });
        let Some(direction) = direction else {
            return;
        };
        ctx.set_cursor_icon(match direction {
            egui::ResizeDirection::North | egui::ResizeDirection::South => {
                egui::CursorIcon::ResizeVertical
            }
            egui::ResizeDirection::East | egui::ResizeDirection::West => {
                egui::CursorIcon::ResizeHorizontal
            }
            egui::ResizeDirection::NorthWest | egui::ResizeDirection::SouthEast => {
                egui::CursorIcon::ResizeNwSe
            }
            egui::ResizeDirection::NorthEast | egui::ResizeDirection::SouthWest => {
                egui::CursorIcon::ResizeNeSw
            }
        });
    }
}

impl NotebookApp {
    /// One pass of the whole window. eframe calls it through `update`, the tests
    /// call it directly with an egui context of their own.
    fn draw(&mut self, ctx: &egui::Context) {
        self.collect_finished_sync();
        if !cfg!(target_os = "macos") {
            self.show_title_bar(ctx);
        }
        self.handle_window_resize(ctx);

        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(Color32::from_rgb(22, 22, 22))
                    .inner_margin(egui::Margin {
                        left: 12.0,
                        right: 4.0,
                        top: 0.0,
                        bottom: 0.0,
                    }),
            )
            .show(ctx, |ui| {
                let available_height = ui.available_height();
                // The built-in scrollbar would run the full height and slip
                // under the floating title bar, so we hide it and paint our
                // own bar (below the header) that drives the same state.
                let bar_width = 16.0;
                // Flush with the header's bottom edge; no gap.
                let bar_top = TITLE_BAR_HEIGHT;
                let mut scroll_output = egui::ScrollArea::vertical()
                    .max_width(ui.available_width() - bar_width)
                    .max_height(available_height)
                    // No scroll animation, anywhere. The caret's line is put where it
                    // belongs on the frame its key lands; an animated view would spend
                    // the next frames gliding after it, and a click that lands far
                    // away jumping is what a plain notebook does.
                    .animated(false)
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .show_viewport(ui, |ui, viewport| {
                        self.editor(ui, viewport, available_height)
                    });
                let viewport = scroll_output.inner_rect;
                let pass = &scroll_output.inner;
                // Scroll for the caret here, once the scroll area is done, by setting
                // its offset. A `scroll_to_rect` made inside it lands a frame late,
                // and egui's alignments put the target's edge at the viewport's edge
                // instead of moving the least. Setting the offset also replaces the
                // text edit's own request, which scrolls only the caret glyph into
                // view and parks the line in the bottom fade.
                let mut target = None;
                if let (true, Some(caret)) = (pass.moved, pass.caret_row) {
                    let delta = caret_scroll(CaretView {
                        caret_top: caret.top(),
                        caret_bottom: caret.bottom(),
                        on_last_row: (caret.bottom() - pass.text_bottom).abs() < 0.5,
                        view_top: pass.view_top,
                        view_bottom: pass.view_bottom,
                    });
                    // The end of the document is the view's furthest offset: the
                    // space kept under the last row is there to rest the line in,
                    // not to scroll into. A caret in the middle of the document
                    // cannot pass it in either direction.
                    let at_end =
                        (pass.offset + pass.text_bottom + CARET_REST_GAP - pass.view_bottom)
                            .max(0.0);
                    let offset = (pass.offset + delta).clamp(0.0, at_end);
                    if scroll_output.state.offset.y != offset {
                        scroll_output.state.offset.y = offset;
                        scroll_output.state.store(ui.ctx(), scroll_output.id);
                    }
                    target = Some(offset);
                }
                // This pass painted the text at the old offset, or without the rows
                // its keys added: the text edit clips to the box it sized before the
                // keys. Have egui paint the frame again from the new state, so the
                // line is in place on the frame its key lands, not half under the
                // bottom edge until the next one.
                let view_moved = target.is_some_and(|offset| (offset - pass.offset).abs() > 0.5);
                let redo = view_moved || pass.outgrew_box;
                if redo {
                    ui.ctx().request_discard("the caret's line moved");
                }
                if target.is_some() || self.trace_repaint {
                    let max = scroll_output.content_size.y - viewport.height();
                    let caret_bottom = pass.caret_row.map_or(f32::NAN, |row| row.bottom());
                    scroll_log(
                        &self.paths.data_dir,
                        &format!(
                            "t={} pass={} painted={:.2} offset={:.2} max={:.2} short={:.2} content={:.2} pen={:.2} pad={:.2} galley={:.2} viewport_bottom={:.2} caret_bottom={:.2} caret_under={:.2} last_row={} chars={} redo={}",
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map_or(0, |d| d.as_millis()),
                            if self.trace_repaint { "repaint" } else { "key" },
                            pass.offset,
                            scroll_output.state.offset.y,
                            max,
                            max - scroll_output.state.offset.y,
                            scroll_output.content_size.y,
                            pass.pen,
                            pass.pad,
                            pass.text_height,
                            pass.view_bottom,
                            caret_bottom,
                            pass.view_bottom - caret_bottom,
                            (caret_bottom - pass.text_bottom).abs() < 0.5,
                            self.text.chars().count(),
                            redo,
                        ),
                    );
                }
                self.trace_repaint = redo && !self.trace_repaint;
                let offset = scroll_output.state.offset.y;
                let caret = pass.caret_row.unwrap_or(egui::Rect::NOTHING);
                let text_height = pass.text_height;
                let changed = pass.response.changed();
                let top_target = top_fade_intensity(offset, viewport, caret);
                let bottom_target =
                    bottom_fade_intensity(offset, viewport, scroll_output.content_size.y, caret);
                // Ease each band towards its target: a band that arrives at full
                // strength the moment text touches the edge reads as a flash, and one
                // that lets go at the end of the document reads as the page jumping.
                let dt = ui.input(|i| i.stable_dt);
                self.fade_top = ease_fade(self.fade_top, top_target, dt);
                self.fade_bottom = ease_fade(self.fade_bottom, bottom_target, dt);
                paint_scroll_fades(ui.painter(), viewport, self.fade_top, self.fade_bottom);
                // Keep painting while a band is still easing.
                if self.fade_top != top_target || self.fade_bottom != bottom_target {
                    ui.ctx().request_repaint_after(Duration::from_millis(16));
                }
                // Painted after the fades so the bar stays crisp at the bottom.
                let bar_rect = egui::Rect::from_min_max(
                    egui::pos2(ui.max_rect().right() - bar_width, bar_top),
                    egui::pos2(ui.max_rect().right(), ui.max_rect().bottom()),
                );
                // The ScrollArea already stored its state for next frame,
                // so push any offset change we made back into the store.
                if paint_custom_scrollbar(
                    ui,
                    bar_rect,
                    &mut scroll_output.state,
                    scroll_output.content_size.y,
                    scroll_output.inner_rect.height(),
                    text_height,
                    &self.text,
                ) {
                    scroll_output.state.store(ui.ctx(), scroll_output.id);
                }
                if changed {
                    self.dirty_since = Some(Instant::now());
                }
                if let Some(t) = self.dirty_since {
                    if t.elapsed() >= SAVE_DEBOUNCE {
                        if self.save_notebook().is_ok() {
                            self.dirty_since = None;
                            self.last_change = Some(Instant::now());
                        } else {
                            // Retry a failed flush without busy-looping the UI thread.
                            ui.ctx().request_repaint_after(Duration::from_millis(400));
                        }
                    } else {
                        ui.ctx().request_repaint_after(SAVE_DEBOUNCE - t.elapsed());
                    }
                }
            });

        if let Some(changed_at) = self.last_change {
            let elapsed = changed_at.elapsed();
            if elapsed >= DEBOUNCE && !self.sync_in_progress {
                self.last_change = None;
                self.start_sync();
            } else if elapsed >= DEBOUNCE {
                // Keep this change queued until the current background push finishes.
                ctx.request_repaint_after(Duration::from_millis(100));
            } else {
                ctx.request_repaint_after(DEBOUNCE - elapsed);
            }
        }
        if self.sync_in_progress {
            // Wake periodically to collect the worker result without blocking the UI.
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        // Centered modal dialog: dimmed backdrop (click outside or Escape to
        // close), rounded surface, full-width fields.
        let mut close_settings = false;
        if self.show_settings {
            egui::Area::new(egui::Id::new("settings_backdrop"))
                .order(egui::Order::Middle)
                .fixed_pos(ctx.screen_rect().min)
                .show(ctx, |ui| {
                    let screen = ctx.screen_rect();
                    ui.painter()
                        .rect_filled(screen, 0.0, Color32::from_black_alpha(140));
                    if ui.allocate_rect(screen, egui::Sense::click()).clicked() {
                        close_settings = true;
                    }
                });

            egui::Window::new("Sync settings")
                .title_bar(false)
                .movable(false)
                .resizable(false)
                .collapsible(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .default_width(440.0)
                .frame(
                    egui::Frame::none()
                        .fill(Color32::from_rgb(30, 30, 30))
                        .stroke(egui::Stroke::new(
                            1.0_f32,
                            Color32::from_rgba_unmultiplied(255, 255, 255, 24),
                        ))
                        .rounding(egui::Rounding::same(10.0))
                        .inner_margin(egui::Margin::same(20.0)),
                )
                .show(ctx, |ui| {
                    // Fixed content width: the window auto-sizes around it.
                    // (INFINITY-width fields would stretch it full-window.)
                    ui.set_width(400.0);
                    if ui.ctx().input(|input| input.key_pressed(egui::Key::Escape)) {
                        close_settings = true;
                    }
                    ui.label(
                        egui::RichText::new("Sync settings")
                            .size(18.0)
                            .color(Color32::from_rgb(232, 232, 232)),
                    );
                    ui.label(
                        egui::RichText::new("Push your notebook to a private GitHub repository.")
                            .weak(),
                    );
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(8.0);
                    ui.label("GitHub repository URL");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.settings.repo_url)
                            .hint_text("https://github.com/you/private-notebook.git"),
                    );
                    ui.add_space(8.0);
                    ui.label("Personal Access Token");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.token_input)
                            .password(true)
                            .hint_text("Fine-grained token with Contents read/write"),
                    );
                    ui.label(
                        egui::RichText::new("Stored in your system keyring.")
                            .weak()
                            .size(11.0),
                    );
                    if let Some(error) = &self.settings_error {
                        ui.add_space(8.0);
                        ui.label(
                            egui::RichText::new(error.as_str())
                                .color(Color32::from_rgb(232, 120, 120))
                                .size(12.0),
                        );
                    }
                    ui.add_space(12.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                        if ui.button("Save").clicked() {
                            match self.save_settings() {
                                Ok(()) => {
                                    self.settings_error = None;
                                    close_settings = true;
                                }
                                Err(error) => {
                                    self.settings_error = Some(error);
                                }
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            self.settings_error = None;
                            close_settings = true;
                        }
                    });
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new("The token is stored in your system keyring.")
                            .weak()
                            .small(),
                    );
                });

            if close_settings {
                self.show_settings = false;
            }
        }

        // Arrow over the title bar: the editor's I-beam must not leak through
        // the floating header. Resize cursors still win on the border.
        let pointer_in_title = ctx.input(|input| {
            input
                .pointer
                .hover_pos()
                .is_some_and(|p| p.y < input.screen_rect().top() + TITLE_BAR_HEIGHT)
        });
        if pointer_in_title {
            ctx.set_cursor_icon(egui::CursorIcon::Default);
        }
        // Last: overrides TextEdit's I-beam when hovering the resize border.
        self.update_resize_cursor(ctx);
    }

    /// The notebook's text inside the scroll area, and the space under it.
    /// `viewport` is the part of the content the scroll area shows this pass.
    fn editor(
        &mut self,
        ui: &mut egui::Ui,
        viewport: egui::Rect,
        available_height: f32,
    ) -> EditorPass {
        // The viewport's edges on screen, as this pass paints the content.
        let content_top = ui.max_rect().top();
        let view_top = content_top + viewport.top();
        let view_bottom = content_top + viewport.bottom();
        // At rest, the first line begins below the floating header. As the
        // document scrolls upward, it passes beneath the header's edge mask.
        ui.add_space(TITLE_BAR_HEIGHT + 8.0);
        // The text edit's box fills the window below the header, so a click
        // in the empty space under a short document lands the caret at the
        // end. Its height leaves no overflow at rest: the header spacer, the
        // box and the trailing pad add up to the window's height.
        let text_box_height =
            (available_height - TITLE_BAR_HEIGHT - 8.0 - BOTTOM_PAD - TEXT_BOX_SLACK).max(0.0);
        let output = TextEdit::multiline(&mut self.text)
            .id(egui::Id::new(EDITOR_ID))
            .code_editor()
            .font(FontId::monospace(14.0))
            .layouter(&mut markdown_layouter)
            .desired_width(ui.available_width())
            .min_size(egui::vec2(0.0, text_box_height))
            .frame(false)
            .hint_text("Start writing...")
            .show(ui);
        // The text as painted, which already holds this pass's keys. The box
        // behind `ui.cursor()` does not: the text edit sized it before applying
        // them.
        let text_bottom = output.galley_pos.y + output.galley.rect.bottom();
        let pen = ui.cursor().min.y;
        let pad = trailing_pad(pen, text_bottom);
        ui.add_space(pad);
        // The whole row the caret is on, not just the glyph: the fades and the
        // scrolling keep the line clear, and the line is wider than the caret.
        let caret_row = output
            .response
            .has_focus()
            .then_some(output.cursor_range)
            .flatten()
            .map(|range| caret_row_rect(output.galley_pos, &output.galley, &range.primary));
        let moved = output.response.changed()
            || ui.input(|i| {
                i.events
                    .iter()
                    .any(|e| matches!(e, egui::Event::Key { pressed: true, .. }))
            });
        let hovered_link = output.response.hover_pos().and_then(|pos| {
            let idx = output
                .galley
                .cursor_from_pos(pos - output.galley_pos)
                .ccursor
                .index;
            // Only count it when the pointer is actually over text.
            output
                .galley
                .rect
                .translate(output.galley_pos.to_vec2())
                .contains(pos)
                .then(|| link_at_char(&self.text, idx))
                .flatten()
        });
        if let Some(url) = hovered_link {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            if output.response.clicked() {
                ui.ctx().open_url(egui::OpenUrl::new_tab(url));
            }
        }
        EditorPass {
            outgrew_box: text_bottom > output.text_clip_rect.bottom() + 0.5,
            response: output.response,
            offset: viewport.top(),
            view_top,
            view_bottom,
            text_bottom,
            text_height: output.galley.rect.height(),
            caret_row,
            moved,
            pen,
            pad,
        }
    }
}

impl eframe::App for NotebookApp {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.draw(ctx);
    }

    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        if self.dirty_since.is_some() {
            let _ = self.save_notebook();
            self.dirty_since = None;
        }
    }
}

pub(crate) fn load_settings(path: &Path) -> Settings {
    fs::read_to_string(path)
        .ok()
        .and_then(|contents| toml::from_str(&contents).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod harness {
    use std::path::PathBuf;

    use super::*;
    use egui::{Event, Key, Modifiers, Rect};

    fn open_app(dir: &Path, text: &str) -> NotebookApp {
        NotebookApp {
            text: text.to_owned(),
            paths: Paths {
                data_dir: dir.to_path_buf(),
                notebook: dir.join("notebook.txt"),
                settings: dir.join("settings.toml"),
                repository: dir.join("repository"),
            },
            settings: Settings::default(),
            token_input: String::new(),
            show_settings: false,
            settings_error: None,
            dirty_since: None,
            last_change: None,
            sync_in_progress: false,
            sync_error: None,
            sync_receiver: None,
            fade_top: 0.0,
            fade_bottom: 0.0,
            trace_repaint: false,
        }
    }

    pub struct Notebook {
        pub ctx: egui::Context,
        pub app: NotebookApp,
        clock: f64,
        screen: Rect,
        ppp: f32,
        dir: PathBuf,
        pub frames: Vec<Painted>,
        pub label: String,
    }

    impl Drop for Notebook {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    /// What one frame put on the screen, read back from the shapes egui
    /// returned: a pass that egui discarded is never measured.
    #[derive(Debug, Clone)]
    pub struct Painted {
        pub what: String,
        pub passes: usize,
        pub chars: usize,
        /// The text's last row on screen, which is the caret's row at the end.
        pub last_row: Rect,
        /// Top of the text on screen: it moves exactly when the view scrolls.
        pub text_top: f32,
        pub viewport_bottom: f32,
        pub clip: Rect,
        /// The caret's row on screen, from the text edit's own state.
        pub caret_row: Option<Rect>,
    }

    impl Painted {
        pub fn under(&self) -> f32 {
            self.viewport_bottom - self.last_row.bottom()
        }
        pub fn visible(&self) -> f32 {
            let top = self.last_row.top().max(self.clip.top());
            let bottom = self
                .last_row
                .bottom()
                .min(self.clip.bottom())
                .min(self.viewport_bottom);
            ((bottom - top) / self.last_row.height()).clamp(0.0, 1.0)
        }
    }

    pub fn key(key: Key, pressed: bool, modifiers: Modifiers) -> Event {
        Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat: false,
            modifiers,
        }
    }

    impl Notebook {
        pub fn open(text: &str, height: f32, ppp: f32) -> Self {
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "think-harness-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            fs::create_dir_all(&dir).unwrap();
            let ctx = egui::Context::default();
            configure_context(&ctx);
            let app = open_app(&dir, text);
            let mut nb = Self {
                ctx,
                app,
                clock: 0.0,
                screen: Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, height)),
                ppp,
                dir,
                frames: Vec::new(),
                label: format!("{height}px window at {ppp}x"),
            };
            for _ in 0..3 {
                nb.frame("start", Vec::new(), Modifiers::NONE);
            }
            nb
        }

        pub fn frame(&mut self, what: &str, events: Vec<Event>, modifiers: Modifiers) -> Painted {
            let mut raw = egui::RawInput::default();
            raw.screen_rect = Some(self.screen);
            raw.focused = true;
            raw.modifiers = modifiers;
            raw.events = events;
            self.clock += 1.0 / 60.0;
            raw.time = Some(self.clock);
            raw.viewports
                .entry(egui::ViewportId::ROOT)
                .or_default()
                .native_pixels_per_point = Some(self.ppp);
            let app = &mut self.app;
            let output = self.ctx.run(raw, |ctx| app.draw(ctx));
            let mut found = None;
            for clipped in &output.shapes {
                if let egui::epaint::Shape::Text(shape) = &clipped.shape {
                    if shape.galley.job.text == self.app.text {
                        found = Some((shape.pos, shape.galley.clone(), clipped.clip_rect));
                    }
                }
            }
            let (pos, galley, clip) = found.expect("the notebook's text was not painted");
            let last_row = galley.rows.last().unwrap().rect.translate(pos.to_vec2());
            let caret_row = egui::TextEdit::load_state(&self.ctx, egui::Id::new("notebook_text"))
                .and_then(|state| state.cursor.char_range())
                .map(|range| {
                    let caret = galley.pos_from_cursor(&galley.from_ccursor(range.primary));
                    Rect::from_min_max(
                        egui::pos2(galley.rect.left(), caret.top()),
                        egui::pos2(galley.rect.right(), caret.bottom()),
                    )
                    .translate(pos.to_vec2())
                });
            let painted = Painted {
                what: what.to_owned(),
                passes: output.platform_output.num_completed_passes,
                chars: self.app.text.chars().count(),
                last_row,
                text_top: pos.y + galley.rect.top(),
                viewport_bottom: self.screen.bottom(),
                clip,
                caret_row,
            };
            self.frames.push(painted.clone());
            painted
        }

        /// One key stroke the way egui-winit reports it: the press and the
        /// text it types in one frame, the repaint egui asks for, the release.
        pub fn stroke(&mut self, what: &str, k: Key, text: Option<&str>) -> Vec<Painted> {
            let mut press = vec![key(k, true, Modifiers::NONE)];
            if let Some(text) = text {
                press.push(Event::Text(text.to_owned()));
            }
            vec![
                self.frame(what, press, Modifiers::NONE),
                self.frame("  repaint", Vec::new(), Modifiers::NONE),
                self.frame(
                    "  release",
                    vec![key(k, false, Modifiers::NONE)],
                    Modifiers::NONE,
                ),
            ]
        }

        pub fn enter(&mut self) -> Vec<Painted> {
            self.stroke("Enter", Key::Enter, None)
        }

        pub fn backspace(&mut self) -> Vec<Painted> {
            self.stroke("Backspace", Key::Backspace, None)
        }

        pub fn type_text(&mut self, text: &str) -> Vec<Painted> {
            let mut out = Vec::new();
            for c in text.chars() {
                let k = if c.is_ascii_alphabetic() {
                    Key::from_name(&c.to_ascii_uppercase().to_string()).unwrap()
                } else if c == ' ' {
                    Key::Space
                } else {
                    Key::Period
                };
                out.extend(self.stroke(&format!("type {c:?}"), k, Some(&c.to_string())));
            }
            out
        }

        pub fn click(&mut self, pos: egui::Pos2) {
            // A second apart, so egui does not count it as a double click.
            self.clock += 1.0;
            self.frame("hover", vec![Event::PointerMoved(pos)], Modifiers::NONE);
            self.frame(
                "click",
                vec![Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Modifiers::NONE,
                }],
                Modifiers::NONE,
            );
            self.frame(
                "  release",
                vec![Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Modifiers::NONE,
                }],
                Modifiers::NONE,
            );
        }

        /// Click into the text, then Ctrl+End: the caret at the end of the
        /// document, the way the user gets there.
        pub fn caret_to_end(&mut self) {
            let pos = egui::pos2(300.0, self.screen.height() / 2.0);
            self.frame("hover", vec![Event::PointerMoved(pos)], Modifiers::NONE);
            self.frame(
                "press",
                vec![Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Modifiers::NONE,
                }],
                Modifiers::NONE,
            );
            self.frame(
                "release",
                vec![Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Modifiers::NONE,
                }],
                Modifiers::NONE,
            );
            let ctrl = Modifiers {
                ctrl: true,
                command: true,
                ..Modifiers::NONE
            };
            self.frame("Ctrl+End", vec![key(Key::End, true, ctrl)], ctrl);
            self.frame("  release", vec![key(Key::End, false, ctrl)], ctrl);
            for _ in 0..3 {
                self.frame("  idle", Vec::new(), Modifiers::NONE);
            }
        }

        pub fn dump(&self) {
            println!("--- {} ---", self.label);
            println!(
                "{:>4} {:<12} {:>2} {:>5} {:>9} {:>8} {:>8} {:>7} {:>5} {:>9} {:>9}",
                "#",
                "event",
                "ps",
                "chars",
                "text_top",
                "row_top",
                "row_bot",
                "under",
                "vis",
                "caret_top",
                "caret_bot"
            );
            for (i, f) in self.frames.iter().enumerate() {
                println!(
                    "{:>4} {:<12} {:>2} {:>5} {:>9.2} {:>8.2} {:>8.2} {:>7.2} {:>5.2} {:>9.2} {:>9.2}",
                    i,
                    f.what,
                    f.passes,
                    f.chars,
                    f.text_top,
                    f.last_row.top(),
                    f.last_row.bottom(),
                    f.under(),
                    f.visible(),
                    f.caret_row.map_or(f32::NAN, |r| r.top()),
                    f.caret_row.map_or(f32::NAN, |r| r.bottom()),
                );
            }
        }
    }

    /// A notebook like the user's: headers, wrapped paragraphs, links, box
    /// drawing and accented characters, blank lines.
    pub fn notebook_text() -> String {
        let mut text = String::new();
        for i in 0..40 {
            if i % 8 == 0 {
                text.push_str(&format!("# Section {i} \u{2502} notes\n"));
            }
            match i % 4 {
                0 => text.push_str("short line\n"),
                1 => text.push_str(
                    "A long paragraph that keeps going past the width of the window, so egui wraps it onto a second row and the row count stops matching the line count. Caf\u{e9} \u{2192} done.\n",
                ),
                2 => text.push_str("see https://example.com/some/path for more\n"),
                _ => text.push('\n'),
            }
        }
        text.push_str("last line");
        text
    }

    pub fn report(nb: &Notebook, from: usize) -> (Vec<String>, Vec<String>) {
        // Problems on Enter frames, and view movement while typing.
        let mut enter = Vec::new();
        let mut moves = Vec::new();
        let frames = &nb.frames[from..];
        for (i, f) in frames.iter().enumerate() {
            if (f.under() - CARET_REST_GAP).abs() > 0.5 || f.visible() < 0.999 {
                enter.push(format!(
                    "frame {} ({}): last row {:.2}px above the edge, {:.0}% visible",
                    from + i,
                    f.what.trim(),
                    f.under(),
                    f.visible() * 100.0
                ));
            }
            if i > 0 {
                let prev = &frames[i - 1];
                let same_rows =
                    (f.last_row.bottom() - f.text_top - (prev.last_row.bottom() - prev.text_top))
                        .abs()
                        < 0.5;
                if same_rows && (f.text_top - prev.text_top).abs() > 0.01 {
                    moves.push(format!(
                        "frame {} ({}): view moved {:+.2}px with no new row",
                        from + i,
                        f.what.trim(),
                        prev.text_top - f.text_top
                    ));
                }
            }
        }
        (enter, moves)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Key;
    use harness::{report, Notebook};

    /// The user's session: reach the end with Ctrl+End, then write.
    fn writing_session(height: f32, ppp: f32) -> (Notebook, usize) {
        let mut nb = Notebook::open(&harness::notebook_text(), height, ppp);
        nb.caret_to_end();
        let from = nb.frames.len();
        for round in 0..4 {
            nb.enter();
            nb.type_text(if round % 2 == 0 { "hello" } else { "ab cd" });
            if round == 2 {
                nb.backspace();
                nb.backspace();
            }
        }
        (nb, from)
    }

    // The reported bug: pressing Enter left the new line half under the bottom
    // edge, and typing made the view jump up and down. Measured from the shapes
    // egui painted, on the frames the keys land, at several window sizes and
    // display scales.
    #[test]
    fn writing_at_the_end_keeps_one_position_and_never_jumps() {
        let mut failures = Vec::new();
        for &(height, ppp) in &[(650.0, 1.0), (604.0, 1.0), (650.0, 1.25), (650.0, 1.5)] {
            let (nb, from) = writing_session(height, ppp);
            let (off_position, moves) = report(&nb, from);
            if !off_position.is_empty() || !moves.is_empty() {
                let label = nb.label.clone();
                nb.dump();
                let mut detail = format!("{label}:");
                for line in off_position {
                    detail.push_str(&format!("\n  off position: {line}"));
                }
                for line in moves {
                    detail.push_str(&format!("\n  jumped: {line}"));
                }
                failures.push(detail);
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }

    // Every Enter rests the line at the same distance from the edge, so the
    // writing position is one place and not a range.
    #[test]
    fn every_enter_rests_the_line_at_the_same_place() {
        let (nb, from) = writing_session(650.0, 1.0);
        let rests: Vec<f32> = nb.frames[from..]
            .iter()
            .filter(|f| f.what.trim() == "Enter" || f.what.trim() == "repaint")
            .map(|f| f.under())
            .collect();
        assert!(rests.len() >= 4, "only {} Enter frames", rests.len());
        for rest in &rests {
            assert!(
                (rest - CARET_REST_GAP).abs() < 0.5,
                "an Enter rested the line {rest:.2}px above the edge, expected {CARET_REST_GAP:.2}, all: {rests:?}",
            );
        }
    }

    // Deleting the last line used to leave a screen of blank under the text: the
    // space below the document was held to the viewport's bottom edge, so it
    // stayed until the writer scrolled up past it. The space follows the text
    // now, so the view is pulled up with the caret's line and the line that is
    // left rests at the writing position, as in a code editor.
    #[test]
    fn deleting_the_last_line_closes_the_space_under_it() {
        let mut nb = Notebook::open(&harness::notebook_text(), 650.0, 1.0);
        nb.caret_to_end();
        nb.enter();
        let line = "temporary line";
        nb.type_text(line);
        let from = nb.frames.len();
        for _ in 0..(line.chars().count() + 2) {
            nb.stroke("Backspace", Key::Backspace, None);
        }

        let off_rest: Vec<String> = nb
            .frames
            .iter()
            .skip(from)
            .filter(|f| f.last_row.is_finite())
            .filter(|f| (f.under() - CARET_REST_GAP).abs() >= 0.5)
            .map(|f| {
                format!(
                    "{} ({}): last row {:.2}px above the edge",
                    f.what.trim(),
                    f.chars,
                    f.under()
                )
            })
            .collect();
        let painted = nb.frames.len() - from;
        nb.dump();
        assert_eq!(
            off_rest,
            Vec::<String>::new(),
            "{painted} frames after the deletion, none resting at the writing position",
        );
    }

    // A long last line wraps into several rows. Deleting from it unwraps a row
    // at a time, and the content shrinks with it: the view follows the text's
    // bottom on every frame, so no space opens under the text and stays there.
    #[test]
    fn deleting_into_a_wrapped_last_line_keeps_the_writing_position() {
        let long = "the quick brown fox jumps over the lazy dog ".repeat(6);
        let text = format!(
            "{}
{}",
            harness::notebook_text(),
            long
        );
        let mut nb = Notebook::open(&text, 650.0, 1.0);
        nb.caret_to_end();
        let from = nb.frames.len();
        for _ in 0..60 {
            nb.stroke("Backspace", Key::Backspace, None);
        }

        let off_rest: Vec<String> = nb
            .frames
            .iter()
            .skip(from)
            .filter(|f| f.last_row.is_finite())
            .filter(|f| (f.under() - CARET_REST_GAP).abs() >= 0.5)
            .map(|f| {
                format!(
                    "{} ({} chars): last row {:.2}px above the edge",
                    f.what.trim(),
                    f.chars,
                    f.under()
                )
            })
            .collect();
        let painted = nb.frames.len() - from;
        nb.dump();
        assert_eq!(
            off_rest,
            Vec::<String>::new(),
            "{painted} frames of deleting, none resting at the writing position",
        );
    }

    /// Watch the caret mid-document: clicks, Enters and arrows near the header
    /// and the bottom edge. Prints, so a report about the fades has numbers.
    #[test]
    fn probe_middle_of_the_document() {
        for &(height, ppp) in &[(650.0, 1.0), (650.0, 1.5)] {
            let mut nb = Notebook::open(&harness::notebook_text(), height, ppp);
            nb.caret_to_end();
            nb.click(egui::pos2(200.0, 300.0));
            nb.type_text("abc");
            nb.enter();
            nb.enter();
            nb.click(egui::pos2(200.0, height - 70.0));
            for _ in 0..4 {
                nb.enter();
            }
            nb.click(egui::pos2(200.0, 70.0));
            nb.type_text("x");
            nb.stroke("ArrowUp", Key::ArrowUp, None);
            nb.stroke("ArrowUp", Key::ArrowUp, None);
            nb.dump();
        }
    }
}
