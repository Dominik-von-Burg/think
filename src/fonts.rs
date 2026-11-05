//! The fonts the notebook paints with: the one it ships, and the system's own
//! caption glyphs.

use std::{path::PathBuf, sync::OnceLock};

use eframe::egui::{self, FontData, FontDefinitions, FontFamily};

pub(crate) const JETBRAINS_MONO: &[u8] = include_bytes!("../assets/JetBrainsMono-Medium.ttf");

/// The family the caption buttons draw their glyphs with.
pub(crate) const CAPTION_FAMILY: &str = "caption";

/// Windows' caption glyphs live in a system icon font, and neither version is
/// redistributable, so the file is read from the Windows font folder at startup:
/// Segoe Fluent Icons on Windows 11, Segoe MDL2 Assets on Windows 10. `None`
/// means the buttons fall back to the strokes this crate draws by hand.
pub(crate) fn caption_font() -> Option<(&'static str, Vec<u8>)> {
    static CACHED: OnceLock<Option<(&'static str, Vec<u8>)>> = OnceLock::new();
    CACHED
        .get_or_init(|| {
            let root = std::env::var("SystemRoot")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from(r"C:\Windows"));
            for (name, file) in [
                ("Segoe Fluent Icons", "SegoeIcons.ttf"),
                ("Segoe MDL2 Assets", "segmdl2.ttf"),
            ] {
                if let Ok(bytes) = std::fs::read(root.join("Fonts").join(file)) {
                    return Some((name, bytes));
                }
            }
            None
        })
        .clone()
}

pub(crate) fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "JetBrains Mono".into(),
        FontData::from_static(JETBRAINS_MONO).tweak(egui::FontTweak {
            scale: 1.0,
            y_offset_factor: 0.0,
            ..Default::default()
        }),
    );
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, "JetBrains Mono".into());
    if let Some((_name, bytes)) = caption_font() {
        fonts
            .font_data
            .insert(CAPTION_FAMILY.into(), FontData::from_owned(bytes));
        fonts.families.insert(
            FontFamily::Name(CAPTION_FAMILY.into()),
            vec![CAPTION_FAMILY.into()],
        );
    }
    ctx.set_fonts(fonts);
}
