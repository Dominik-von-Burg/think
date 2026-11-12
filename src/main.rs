#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

//! Think: a private notebook that commits and pushes itself.
//!
//! The crate is one binary, split by what a frame needs: [[app]] builds the frame,
//! [[scroll]] decides where the caret's line rests, [[title_bar]] draws the window
//! chrome, [[markdown]] lays text out, [[sync]] talks to the remote, [[trace]]
//! writes the scroll log, [[paths]] finds the data directory, [[fonts]] installs
//! the font, and [[win_chrome]] handles the Windows message loop.

mod app;
mod fonts;
mod keyring_store;
#[cfg(target_os = "linux")]
mod linux_caption;
#[cfg(target_os = "linux")]
mod linux_window;
mod markdown;
mod mcp;
mod notebook;
mod paths;
mod scroll;
mod sync;
mod title_bar;
mod trace;
#[cfg(target_os = "windows")]
mod win_chrome;

use eframe::{
    egui::{self, Color32},
    NativeOptions,
};

use crate::app::NotebookApp;
use crate::fonts::install_fonts;

pub(crate) const APP_NAME: &str = "Think";

pub(crate) const APP_ICON_RGBA: &[u8] = include_bytes!("../assets/think-icon.rgba");

/// Fonts, colors and spacing, everything the app sets on the egui context. The
/// tests' notebook goes through it too, so it lays text out like the app.
pub(crate) fn configure_context(ctx: &egui::Context) {
    install_fonts(ctx);
    egui_extras::install_image_loaders(ctx);
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = Color32::from_rgb(22, 22, 22);
    visuals.window_fill = Color32::from_rgb(22, 22, 22);
    visuals.extreme_bg_color = Color32::from_rgb(22, 22, 22);
    visuals.faint_bg_color = Color32::from_rgb(30, 30, 30);
    visuals.override_text_color = Some(Color32::from_rgb(232, 232, 232));
    ctx.set_visuals(visuals);
    let mut style = (*ctx.style()).clone();
    style.spacing.scroll.floating = true;
    style.spacing.scroll.floating_width = 3.0;
    style.spacing.scroll.floating_allocated_width = 4.0;
    style.spacing.scroll.bar_width = 6.0;
    ctx.set_style(style);
    ctx.tessellation_options_mut(|tess| {
        tess.round_text_to_pixels = true;
        tess.feathering = true;
        tess.feathering_size_in_pixels = 1.0;
    });
}

#[cfg(target_os = "windows")]
pub(crate) fn set_windows_title_bar_colors(cc: &eframe::CreationContext<'_>) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;

    let Ok(handle) = cc.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(window) = handle.as_raw() else {
        return;
    };

    // These Windows 10/11 DWM attributes color the native caption while keeping
    // normal resizing, snapping, and the system close/minimize/maximize buttons.
    const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
    const DWMWA_CAPTION_COLOR: u32 = 35;
    const DWMWA_TEXT_COLOR: u32 = 36;
    const DWMWCP_ROUND: u32 = 2;
    let caption_color: u32 = 0x0016_1616;
    let text_color: u32 = 0x00E8_E8E8;
    unsafe {
        DwmSetWindowAttribute(
            window.hwnd.get() as _,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &DWMWCP_ROUND as *const u32 as _,
            std::mem::size_of::<u32>() as u32,
        );
        DwmSetWindowAttribute(
            window.hwnd.get() as _,
            DWMWA_CAPTION_COLOR,
            &caption_color as *const u32 as _,
            std::mem::size_of::<u32>() as u32,
        );
        DwmSetWindowAttribute(
            window.hwnd.get() as _,
            DWMWA_TEXT_COLOR,
            &text_color as *const u32 as _,
            std::mem::size_of::<u32>() as u32,
        );
    }
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn set_windows_title_bar_colors(_: &eframe::CreationContext<'_>) {}

pub(crate) fn app_icon() -> egui::IconData {
    egui::IconData {
        rgba: APP_ICON_RGBA.to_vec(),
        width: 256,
        height: 256,
    }
}

pub(crate) fn main() {
    if std::env::args().any(|arg| arg == "--mcp") {
        eprintln!("MCP now runs over HTTP. Open Think and approve access in Settings.");
        std::process::exit(2);
    }

    let viewport = egui::ViewportBuilder::default()
        .with_inner_size([900.0, 650.0])
        .with_min_inner_size([400.0, 300.0])
        .with_icon(app_icon())
        .with_decorations(cfg!(target_os = "macos"))
        .with_transparent(cfg!(target_os = "linux"));

    if let Err(e) = eframe::run_native(
        APP_NAME,
        NativeOptions {
            viewport,
            // Keep glyphs sharp. MSAA softens egui's text atlas.
            multisampling: 0,
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(NotebookApp::new(cc)?))),
    ) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
