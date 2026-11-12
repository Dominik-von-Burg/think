//! The window's title bar: the buttons drawn in it and the space they take.

#[cfg(any(target_os = "windows", test))]
use egui::Color32;

#[cfg(any(target_os = "windows", test))]
use crate::fonts::{caption_font, CAPTION_FAMILY};

#[cfg(target_os = "windows")]
use crate::win_chrome;

pub(crate) const TITLE_BAR_HEIGHT: f32 = 36.0;

/// Aligns the "Think" label with the text edit's inner margin.
#[cfg(target_os = "windows")]
pub(crate) const TITLE_X_NUDGE: f32 = 4.0;

#[derive(Clone, Copy)]
pub(crate) enum TitleBarIcon {
    Settings,
}

pub(crate) fn lucide_svg(icon: TitleBarIcon) -> (&'static str, &'static str) {
    match icon {
        TitleBarIcon::Settings => (
            "settings",
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#d6d6d6" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M9.671 4.136a2.34 2.34 0 0 1 4.659 0 2.34 2.34 0 0 0 3.319 1.915 2.34 2.34 0 0 1 2.33 4.033 2.34 2.34 0 0 0 0 3.831 2.34 2.34 0 0 1-2.33 4.033 2.34 2.34 0 0 0-3.319 1.915 2.34 2.34 0 0 1-4.659 0 2.34 2.34 0 0 0-3.32-1.915 2.34 2.34 0 0 1-2.33-4.033 2.34 2.34 0 0 0 0-3.831A2.34 2.34 0 0 1 6.35 6.051a2.34 2.34 0 0 0 3.319-1.915"/><circle cx="12" cy="12" r="3"/></svg>"##,
        ),
    }
}

pub(crate) fn lucide_image(icon: TitleBarIcon) -> egui::Image<'static> {
    let (name, svg) = lucide_svg(icon);
    egui::Image::from_bytes(format!("bytes://think/lucide-{name}.svg"), svg.as_bytes())
        .fit_to_exact_size(egui::vec2(16.0, 16.0))
}

#[cfg(any(target_os = "windows", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CaptionButton {
    Minimize,
    Maximize,
    Restore,
    Close,
}

#[cfg(target_os = "windows")]
pub(crate) fn caption_button(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    kind: CaptionButton,
) -> egui::Response {
    let response = ui.allocate_rect(rect, egui::Sense::click());
    let mut hovered = response.hovered();
    let mut pressed = response.is_pointer_button_down_on();
    if matches!(kind, CaptionButton::Maximize | CaptionButton::Restore) {
        (hovered, pressed) = win_chrome::max_button_state();
    }

    // Native sizes: 46px wide, flush, no gaps.
    if hovered || pressed {
        let (fill_rect, rounding, fill) = if pressed {
            caption_hover_pressed(rect, kind)
        } else {
            caption_hover(rect, kind)
        };
        ui.painter().rect_filled(fill_rect, rounding, fill);
    }

    let icon = match kind {
        CaptionButton::Close => Color32::WHITE,
        _ if hovered || pressed => Color32::WHITE,
        _ => Color32::from_rgb(200, 200, 200),
    };
    paint_caption_glyph(ui.painter(), rect, kind, icon);
    response
}

/// The fill a caption button shows while hovered: the whole button, square, on
/// Windows 11 24H2. The earlier corner radius was an older Windows 11 look.
/// Close fills red; minimise and maximise fill with a light wash.
#[cfg(any(target_os = "windows", test))]
fn caption_hover(rect: egui::Rect, kind: CaptionButton) -> (egui::Rect, f32, Color32) {
    let fill = match kind {
        CaptionButton::Close => Color32::from_rgb(196, 43, 28),
        _ => Color32::from_rgba_unmultiplied(255, 255, 255, 22),
    };
    (rect, 0.0, fill)
}

/// While the button is held down: the same shape, a shade deeper.
#[cfg(any(target_os = "windows", test))]
fn caption_hover_pressed(rect: egui::Rect, kind: CaptionButton) -> (egui::Rect, f32, Color32) {
    let fill = match kind {
        CaptionButton::Close => Color32::from_rgb(139, 29, 20),
        _ => Color32::from_rgba_unmultiplied(255, 255, 255, 14),
    };
    (rect, 0.0, fill)
}

/// The glyph Windows uses for this button, from its own caption font:
/// ChromeMinimize, ChromeMaximize, ChromeRestore, ChromeClose.
#[cfg(any(target_os = "windows", test))]
fn caption_glyph(kind: CaptionButton) -> char {
    match kind {
        CaptionButton::Minimize => '\u{e921}',
        CaptionButton::Maximize => '\u{e922}',
        CaptionButton::Restore => '\u{e923}',
        CaptionButton::Close => '\u{e8bb}',
    }
}

/// 10pt at 96dpi, the size Windows sets the caption glyphs at.
#[cfg(any(target_os = "windows", test))]
const CAPTION_GLYPH_SIZE: f32 = 13.0;

#[cfg(any(target_os = "windows", test))]
fn caption_glyph_galley(
    painter: &egui::Painter,
    kind: CaptionButton,
    color: Color32,
) -> Option<std::sync::Arc<egui::Galley>> {
    caption_font()?;
    let mut job = egui::text::LayoutJob::default();
    job.append(
        caption_glyph(kind).to_string().as_str(),
        0.0,
        egui::TextFormat {
            font_id: egui::FontId::new(
                CAPTION_GLYPH_SIZE,
                egui::FontFamily::Name(CAPTION_FAMILY.into()),
            ),
            color,
            ..Default::default()
        },
    );
    let galley = painter.layout_job(job);
    (galley.rect.width() > 0.0 && galley.rect.height() > 0.0).then_some(galley)
}

#[cfg(target_os = "windows")]
fn paint_caption_glyph(
    painter: &egui::Painter,
    rect: egui::Rect,
    kind: CaptionButton,
    color: Color32,
) {
    // Windows paints its caption buttons from the system's icon font, and the
    // custom frame puts that region in our hands, so the same glyphs are drawn
    // from the same font. Without the font on the machine, the strokes below.
    if let Some(galley) = caption_glyph_galley(painter, kind, color) {
        let pos = rect.center() - galley.rect.center().to_vec2();
        painter.galley(painter.round_pos_to_pixels(pos), galley, color);
    } else {
        paint_caption_stroke(painter, rect, kind, color);
    }
}

/// The caption glyphs drawn by hand: thin 1px Win11-style strokes on a 10x10
/// box, not heavy 2px strokes. Snap to the pixel grid so 1px strokes stay crisp
/// and all glyphs line up.
#[cfg(target_os = "windows")]
fn paint_caption_stroke(
    painter: &egui::Painter,
    rect: egui::Rect,
    kind: CaptionButton,
    color: Color32,
) {
    // Thin 1px Win11-style glyphs on a 10x10 box, not heavy 2px strokes.
    // Snap to the pixel grid so 1px strokes stay crisp and all glyphs line up.
    let c = painter.round_pos_to_pixel_center(rect.center());
    let stroke = egui::Stroke::new(1.0_f32, color);
    match kind {
        CaptionButton::Minimize => {
            painter.line_segment(
                [egui::pos2(c.x - 5.0, c.y), egui::pos2(c.x + 5.0, c.y)],
                stroke,
            );
        }
        CaptionButton::Maximize => {
            painter.rect_stroke(
                egui::Rect::from_center_size(c, egui::vec2(10.0, 10.0)),
                0.0,
                stroke,
            );
        }
        CaptionButton::Restore => {
            let front = egui::Rect::from_center_size(
                egui::pos2(c.x - 1.0, c.y + 1.0),
                egui::vec2(8.0, 8.0),
            );
            painter.rect_stroke(front, 0.0, stroke);
            // Back window: top + right edges peeking above the front square.
            painter.line_segment(
                [
                    egui::pos2(front.left() + 2.0, front.top() - 2.5),
                    egui::pos2(front.right() + 2.0, front.top() - 2.5),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(front.right() + 2.0, front.top() - 2.5),
                    egui::pos2(front.right() + 2.0, front.top() + 1.5),
                ],
                stroke,
            );
        }
        CaptionButton::Close => {
            painter.line_segment(
                [
                    egui::pos2(c.x - 5.0, c.y - 5.0),
                    egui::pos2(c.x + 5.0, c.y + 5.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(c.x - 5.0, c.y + 5.0),
                    egui::pos2(c.x + 5.0, c.y - 5.0),
                ],
                stroke,
            );
        }
    }
}

pub(crate) fn title_bar_icon_button(ui: &mut egui::Ui, icon: TitleBarIcon) -> egui::Response {
    ui.add_sized(
        [34.0, 30.0],
        egui::Button::image(lucide_image(icon)).frame(false),
    )
}

pub(crate) fn sync_indicator(ui: &mut egui::Ui) -> egui::Response {
    let refresh = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#bdbdbd" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/><path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16"/><path d="M8 16H3v5"/></svg>"##;
    let image = egui::Image::from_bytes("bytes://think/lucide-refresh-cw.svg", refresh.as_bytes())
        .fit_to_exact_size(egui::vec2(16.0, 16.0));
    ui.add_sized([28.0, 30.0], image)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The buttons draw Windows' own glyphs, so every one of them has to lay
    /// out. An empty galley means the font has no such glyph, and the caption
    /// would fall back to the strokes this crate draws by hand.
    #[test]
    fn the_caption_glyphs_lay_out() {
        let (name, _) = match caption_font() {
            Some(found) => found,
            None => {
                eprintln!("no Windows caption font here: the strokes are used");
                return;
            }
        };
        let ctx = egui::Context::default();
        crate::fonts::install_fonts(&ctx);
        let mut widths: Vec<f32> = Vec::new();
        let _out = ctx.run(egui::RawInput::default(), |ctx| {
            let painter =
                ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::NULL));
            widths = [
                CaptionButton::Minimize,
                CaptionButton::Maximize,
                CaptionButton::Restore,
                CaptionButton::Close,
            ]
            .into_iter()
            .filter_map(|kind| {
                caption_glyph_galley(&painter, kind, Color32::WHITE).map(|g| g.rect.width())
            })
            .collect::<Vec<f32>>();
        });
        assert_eq!(widths.len(), 4, "a caption glyph did not lay out");
        // Windows' caption font is a monospace icon font: every glyph advances
        // by its em, so four advances of about 13px at 13px mean the family
        // resolved. A fallback font substitution would advance differently.
        for width in &widths {
            assert!(
                (CAPTION_GLYPH_SIZE * 0.7..=CAPTION_GLYPH_SIZE * 1.6).contains(width),
                "a {name} glyph advanced {width:.1}px at {CAPTION_GLYPH_SIZE:.0}px: {widths:?}",
            );
        }
    }
    // The fills square, flush, over the whole button: Windows 11 24H2 draws the
    // caption hovers without a corner radius, and close over the full button.
    #[test]
    fn the_caption_hover_fills_the_button_square() {
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(46.0, 36.0));
        for kind in [
            CaptionButton::Minimize,
            CaptionButton::Maximize,
            CaptionButton::Restore,
            CaptionButton::Close,
        ] {
            for (fill_rect, rounding, _) in
                [caption_hover(rect, kind), caption_hover_pressed(rect, kind)]
            {
                assert_eq!(fill_rect, rect, "{kind:?} hover does not fill the button");
                assert_eq!(rounding, 0.0, "{kind:?} hover has rounded corners");
            }
        }
        assert_eq!(
            caption_hover(rect, CaptionButton::Close).2,
            Color32::from_rgb(196, 43, 28),
            "close must hover red",
        );
        assert_eq!(
            caption_hover_pressed(rect, CaptionButton::Close).2,
            Color32::from_rgb(139, 29, 20),
            "close must use a deeper red while pressed",
        );
    }
}
