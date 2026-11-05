//! Where the caret's line rests in the view, and the fades at its edges.
//!
//! The writing position: the document's last row rests a fixed distance above the
//! window's bottom edge, so the line a key lands on is always in the same place.
//! The fades ramp in with the scroll position and leave the caret's line alone.

use egui::Color32;

use crate::markdown::is_markdown_header;
use crate::title_bar::TITLE_BAR_HEIGHT;

/// Trailing space in the document. Kept shallow: the text edit's box covers the
/// rest of the window, so a click at the bottom edge puts the caret at the end.
pub(crate) const BOTTOM_PAD: f32 = 12.0;

/// Depth of the bottom fade. Kept near one text row: deep enough to hint that
/// text continues past the edge, shallow enough that the edge of the page does
/// not turn into a wall.
pub(crate) const SCROLL_FADE_HEIGHT: f32 = 24.0;

/// Peak opacity of each fade at the window's edge. The bottom band stays
/// translucent so the text under it reads as receding, not as deleted; the top
/// band has to hide text running under the floating title, so it is stronger.
pub(crate) const FADE_BOTTOM_ALPHA: f32 = 0.55;

pub(crate) const FADE_TOP_ALPHA: f32 = 0.85;

/// Scroll distance over which a fade eases in or out, so no band ever switches
/// on at full strength.
pub(crate) const FADE_RAMP: f32 = 24.0;

/// Gap kept between the caret's line and the inner edge of a fade band, so the
/// line being typed is never shaded by a fade.
pub(crate) const CARET_FADE_GAP: f32 = 8.0;

/// The writing position: how far the bottom of the document's last row rests
/// above the window's bottom edge, the same after every Enter.
pub(crate) const CARET_REST_GAP: f32 = 20.0;

/// The notebook's text edit, under a fixed id so its caret can be read back.
pub(crate) const EDITOR_ID: &str = "notebook_text";

/// The space under the text, so the content's furthest offset is the writing
/// position: the text's last row, plus `CARET_REST_GAP`.
///
/// It is measured from the text, not from the text edit's box, which lags a row
/// behind the keys: too short on the pass Enter lands, too tall on the pass a
/// Backspace joins two lines. Measuring from the box props the content open by
/// that row, so deleting the last line leaves a band of blank under the text
/// that only goes away when the writer scrolls up past it. Measuring from the
/// text closes the space as the text shrinks, and the view is pulled up with the
/// caret's line, as in a code editor.
///
/// `pen` is where the text edit's box ends, `text_bottom` the bottom of the text
/// as painted.
pub(crate) fn trailing_pad(pen: f32, text_bottom: f32) -> f32 {
    (text_bottom + CARET_REST_GAP - pen).max(0.0)
}

/// The caret's row and the viewport, on screen as one pass painted them.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CaretView {
    pub(crate) caret_top: f32,
    pub(crate) caret_bottom: f32,
    /// The caret sits on the document's last row.
    pub(crate) on_last_row: bool,
    pub(crate) view_top: f32,
    pub(crate) view_bottom: f32,
}

/// How far to scroll, positive down, to put the caret's line where the writer
/// expects it.
///
/// On the document's last row the line belongs at the writing position, so the
/// delta is the distance to that place: the line Enter adds lands there, typing
/// leaves it there, a Backspace that joins two lines stays there, and when a
/// deletion shortens the document the line that is left comes back down to it.
///
/// Elsewhere the view moves the least that keeps the line out of both fade
/// bands, plus a gap.
pub(crate) fn caret_scroll(view: CaretView) -> f32 {
    let top_limit = view.view_top + CARET_TOP_CLEARANCE;
    if view.on_last_row {
        return view.caret_bottom - (view.view_bottom - CARET_REST_GAP);
    }
    let bottom_limit = view.view_bottom - CARET_BOTTOM_CLEARANCE;
    if view.caret_bottom > bottom_limit {
        view.caret_bottom - bottom_limit
    } else if view.caret_top < top_limit {
        view.caret_top - top_limit
    } else {
        0.0
    }
}

/// What one pass of the editor painted, in screen points, for the scroll
/// decision made once the scroll area is done.
pub(crate) struct EditorPass {
    pub(crate) response: egui::Response,
    /// The scroll offset this pass was painted at.
    pub(crate) offset: f32,
    pub(crate) view_top: f32,
    pub(crate) view_bottom: f32,
    /// Bottom of the text's last row.
    pub(crate) text_bottom: f32,
    pub(crate) text_height: f32,
    /// The whole row the caret is on, while the text edit has focus.
    pub(crate) caret_row: Option<egui::Rect>,
    /// A key went down or the text changed: the caret may have moved.
    pub(crate) moved: bool,
    /// The text outgrew the box the text edit sized from the text before this
    /// pass's keys. The text edit clips its painting to that box, so the rows the
    /// keys added (the new line after Enter, with the caret on it) are missing
    /// from this pass's picture.
    pub(crate) outgrew_box: bool,
    /// End of the text edit's box, and the trailing space after it, for the trace.
    pub(crate) pen: f32,
    pub(crate) pad: f32,
}

/// Space kept under a caret in the middle of the document when scrolling it
/// into view: the bottom fade band plus a gap, so the line being read or typed
/// never rests in the shade.
pub(crate) const CARET_BOTTOM_CLEARANCE: f32 = SCROLL_FADE_HEIGHT + CARET_FADE_GAP;

/// Gap kept above the caret so its line clears the header's top fade, which is
/// as deep as the floating title zone.
pub(crate) const CARET_TOP_CLEARANCE: f32 = TITLE_BAR_HEIGHT + 8.0 + CARET_FADE_GAP;

/// The text edit's box is sized this much short of the space under the header.
/// egui adds the text edit's margins and the layout's item spacing on top of
/// `min_size`, so an exact fit overflows the window by a few px at rest: room
/// the view can scroll away, leaving a band at the bottom edge that a click
/// cannot put a caret in.
pub(crate) const TEXT_BOX_SLACK: f32 = 6.0;

/// Paint one edge fade. `peak` is the opacity at the window's edge and
/// `intensity` scales the whole band, so a band can ease in and out with the
/// scroll position instead of switching on at full strength.
pub(crate) fn paint_fade(
    painter: &egui::Painter,
    band: egui::Rect,
    opaque_at_top: bool,
    peak: f32,
    intensity: f32,
) {
    const ROWS: u32 = 12;
    let mut mesh = egui::Mesh::default();
    for i in 0..=ROWS {
        let t = i as f32 / ROWS as f32;
        let y = band.top() + (band.bottom() - band.top()) * t;
        let a = if opaque_at_top { 1.0 - t } else { t };
        let mut a = a * a * (3.0 - 2.0 * a); // smoothstep
        if opaque_at_top {
            // Hold opacity longer so scrolled text can't show through
            // behind the title bar and caption buttons.
            a = a.powf(0.6);
        }
        let a = (a * peak * intensity).clamp(0.0, 1.0);
        let c = Color32::from_rgba_unmultiplied(22, 22, 22, (a * 255.0 + 0.5) as u8);
        mesh.colored_vertex(egui::pos2(band.left(), y), c);
        mesh.colored_vertex(egui::pos2(band.right(), y), c);
    }
    for i in 0..ROWS {
        let k = (i * 2) as u32;
        mesh.add_triangle(k, k + 1, k + 2);
        mesh.add_triangle(k + 1, k + 3, k + 2);
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// The whole row the caret is on, on screen, so the scrolling can keep the line
/// clear of the fades and the fades can find the line back. egui reports the
/// caret in the galley's own coordinates, with the row's full height; moving it
/// by `galley_pos` puts it where it was painted.
pub(crate) fn caret_row_rect(
    galley_pos: egui::Pos2,
    galley: &egui::Galley,
    cursor: &egui::epaint::text::cursor::Cursor,
) -> egui::Rect {
    let caret = galley.pos_from_cursor(cursor);
    egui::Rect::from_min_max(
        egui::pos2(galley.rect.left(), caret.top()),
        egui::pos2(galley.rect.right(), caret.bottom()),
    )
    .translate(galley_pos.to_vec2())
}

/// Depth of the header's fade, and of the bottom fade, for a viewport. The
/// caret clearances are derived from these, so they must stay in sync.
pub(crate) fn top_fade_height(viewport: egui::Rect) -> f32 {
    (TITLE_BAR_HEIGHT + 8.0).min(viewport.height() / 3.0)
}

pub(crate) fn bottom_fade_height(viewport: egui::Rect) -> f32 {
    SCROLL_FADE_HEIGHT.min(viewport.height() / 3.0)
}

/// Put the caret's global rect in the scroll area's content coordinates, where
/// the visible window spans `offset .. offset + viewport height`.
pub(crate) fn caret_in_viewport(
    offset: f32,
    viewport: egui::Rect,
    caret: egui::Rect,
) -> (f32, f32) {
    (
        caret.top() - viewport.min.y + offset,
        caret.bottom() - viewport.min.y + offset,
    )
}

/// Ease a band's strength towards its target. Exponential smoothing, so the
/// glide looks the same at any frame rate, with the last step snapped so a band
/// comes to rest exactly on its target. The step per frame is bounded, which is
/// what keeps a band from popping.
pub(crate) fn ease_fade(current: f32, target: f32, dt: f32) -> f32 {
    let diff = target - current;
    if diff.abs() < 0.004 {
        return target;
    }
    let k = 1.0 - (-18.0 * dt.max(0.0)).exp();
    (current + diff * k).clamp(0.0, 1.0)
}

/// A fade is decoration over text leaving the view, and must never shade the
/// line the caret is on: that is the line being read and typed. Where the view
/// cannot scroll the caret out of the band, the band goes to zero. The request
/// in the editor keeps the caret clear of both bands, but the offset read at
/// paint time predates that request, and at the end of the document the offset
/// is already at its maximum, so the line rests inside the bottom band.
///
/// The returned strength ramps over the last [`FADE_RAMP`] px of the scroll
/// range, so the band eases in as text starts to leave the view and eases out
/// as the edge of the document arrives.
pub(crate) fn top_fade_intensity(offset: f32, viewport: egui::Rect, caret: egui::Rect) -> f32 {
    if offset <= 0.0 {
        return 0.0;
    }
    if caret_in_band(offset, viewport, caret, true) {
        return 0.0;
    }
    (offset / FADE_RAMP).min(1.0)
}

/// See [`top_fade_intensity`].
pub(crate) fn bottom_fade_intensity(
    offset: f32,
    viewport: egui::Rect,
    content_height: f32,
    caret: egui::Rect,
) -> f32 {
    // Content hanging below the viewport's bottom edge: nothing below means
    // nothing is leaving the view, so there is nothing to fade.
    let below = content_height - (offset + viewport.height());
    if below <= 0.0 {
        return 0.0;
    }
    if caret_in_band(offset, viewport, caret, false) {
        return 0.0;
    }
    (below / FADE_RAMP).min(1.0)
}

/// True when the caret's line overlaps the band at one edge of the viewport.
pub(crate) fn caret_in_band(
    offset: f32,
    viewport: egui::Rect,
    caret: egui::Rect,
    from_top: bool,
) -> bool {
    let (caret_top, caret_bottom) = caret_in_viewport(offset, viewport, caret);
    let in_view = caret_bottom > offset && caret_top < offset + viewport.height();
    let in_band = if from_top {
        caret_top < offset + top_fade_height(viewport)
    } else {
        caret_bottom > offset + viewport.height() - bottom_fade_height(viewport)
    };
    in_view && in_band
}

/// `peak` is the opacity the band reaches at the window's edge.
pub(crate) fn paint_scroll_fades(painter: &egui::Painter, rect: egui::Rect, top: f32, bottom: f32) {
    // The top fade covers the whole floating title zone (bar + spacing)
    // so scrolled text is fully masked behind the title and buttons.
    let top_height = top_fade_height(rect);
    let height = bottom_fade_height(rect);
    if top_height <= 0.0 {
        return;
    }

    if top > 0.001 {
        paint_fade(
            painter,
            egui::Rect::from_min_max(
                egui::pos2(rect.left(), rect.top()),
                egui::pos2(rect.right(), rect.top() + top_height),
            ),
            true,
            FADE_TOP_ALPHA,
            top,
        );
    }

    if bottom > 0.001 {
        paint_fade(
            painter,
            egui::Rect::from_min_max(
                egui::pos2(rect.left(), rect.bottom() - height),
                egui::pos2(rect.right(), rect.bottom()),
            ),
            false,
            FADE_BOTTOM_ALPHA,
            bottom,
        );
    }
}

/// Scrollbar for the notebook, starting below the floating title bar.
/// Drives the ScrollArea's own `ScrollState`, so wheel scrolling over the
/// content and dragging the thumb stay in sync.
pub(crate) fn paint_custom_scrollbar(
    ui: &egui::Ui,
    rect: egui::Rect,
    state: &mut egui::scroll_area::State,
    content_height: f32,
    viewport_height: f32,
    text_height: f32,
    text: &str,
) -> bool {
    // The bar is shorter than the scroll area's viewport (it starts below
    // the floating header), so map content coordinates onto the bar and let
    // the thumb represent the viewport's full height. Clamping to the scroll
    // area's own max offset keeps the thumb from snapping back at the ends.
    let bar_height = rect.height();
    let max_offset = (content_height - viewport_height).max(0.0);
    let offset = state.offset.y.clamp(0.0, max_offset);
    let scale = if content_height > 0.0 {
        bar_height / content_height
    } else {
        0.0
    };

    let thumb_height = if max_offset <= 0.0 {
        bar_height
    } else {
        (viewport_height * scale).min(bar_height).max(32.0)
    };
    let thumb_rect = egui::Rect::from_min_size(
        egui::pos2(rect.left(), rect.top() + offset * scale),
        egui::vec2(rect.width(), thumb_height),
    );

    let response = ui.interact(
        rect,
        egui::Id::new("notebook_scrollbar"),
        egui::Sense::click_and_drag(),
    );
    let mut changed = false;
    let mut new_offset = offset;
    if response.dragged() {
        if scale > 0.0 {
            new_offset = (offset + response.drag_delta().y / scale).clamp(0.0, max_offset);
        }
    } else if response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            if scale > 0.0 {
                // Center the thumb on the click.
                new_offset =
                    ((pos.y - rect.top()) / scale - viewport_height / 2.0).clamp(0.0, max_offset);
            }
        }
    } else if response.hovered() {
        // The bar sits outside the ScrollArea's rect, so wheel over it is
        // not consumed by the scroll area itself.
        let scroll = ui.ctx().input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            new_offset = (offset + scroll).clamp(0.0, max_offset);
        }
    }
    if (new_offset - offset).abs() > 0.01 {
        state.offset = egui::vec2(state.offset.x, new_offset);
        changed = true;
        ui.ctx().request_repaint();
    }

    let painter = ui.painter();
    // VSCode-style: no track background. A thin line on the left edge carries
    // the header markers; the thumb is centered between the line and the
    // window's right edge.
    let line_x = rect.left() + 0.5;
    let line_color = Color32::from_rgba_unmultiplied(255, 255, 255, 14);
    painter.rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(line_x, rect.top()),
            egui::pos2(line_x + 1.0, rect.bottom()),
        ),
        0.0,
        line_color,
    );
    // Top edge: closes the scrollbar container, from the line all the way to
    // the window's right edge. Same style as the window's right border.
    let window_right = ui.ctx().screen_rect().right();
    painter.rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(line_x, rect.top()),
            egui::pos2(window_right, rect.top() + 1.0),
        ),
        0.0,
        Color32::from_rgb(54, 54, 54),
    );

    // Thumb: centered between the line and the window's right edge, shifted
    // 2px right.
    let thumb_width = 12.0;
    let center_x = (line_x + 0.5 + rect.right()) / 2.0 + 2.0;
    let thumb = egui::Rect::from_center_size(
        egui::pos2(center_x, thumb_rect.center().y),
        egui::vec2(thumb_width, thumb_rect.height()),
    );
    let thumb_color = if response.dragged() {
        Color32::from_rgba_unmultiplied(255, 255, 255, 90)
    } else if response.hovered() {
        Color32::from_rgba_unmultiplied(255, 255, 255, 64)
    } else {
        Color32::from_rgba_unmultiplied(255, 255, 255, 38)
    };
    painter.rect_filled(thumb, 3.0, thumb_color);

    // Header markers: one per markdown header, always visible on the line.
    let line_count = text.lines().count();
    if line_count > 0 && content_height > TITLE_BAR_HEIGHT + 8.0 {
        // The text is one uniform line height per line, so the line height
        // falls out of the text's measured height. Not the content's: the
        // space under the text grows to hold the view still.
        let line_height = text_height / line_count as f32;
        let mark = Color32::from_rgba_unmultiplied(140, 180, 232, 110);
        for (i, line) in text.lines().enumerate() {
            if !is_markdown_header(line) {
                continue;
            }
            let y = rect.top() + (TITLE_BAR_HEIGHT + 8.0 + (i as f32 + 0.5) * line_height) * scale;
            if !(rect.top()..=rect.bottom()).contains(&y) {
                continue;
            }
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(line_x + 4.0, y - 0.5),
                    egui::pos2(rect.right() + 1.0, y + 0.5),
                ),
                0.0,
                mark,
            );
        }
    }

    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    use eframe::egui;

    // The bands carry the writing area: they have to ramp in and out with the
    // scroll position, not switch on at full strength the moment text touches
    // the edge, and they have to be gone at the end of the document.
    #[test]
    fn fade_bands_ramp_with_the_scroll_position() {
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let content = 1000.0f32;
        let no_caret = egui::Rect::NOTHING;
        let top: Vec<f32> = (0..=6)
            .map(|i| top_fade_intensity(i as f32 * FADE_RAMP / 3.0, viewport, no_caret))
            .collect();
        // Walking back from the end of the document, the band climbs.
        let last = content - viewport.height();
        let bottom: Vec<f32> = (0..=6)
            .map(|i| {
                bottom_fade_intensity(
                    last - i as f32 * FADE_RAMP / 3.0,
                    viewport,
                    content,
                    no_caret,
                )
            })
            .collect();
        assert!(top[0] <= 0.001, "no fade at the top of the document");
        assert!(
            top.windows(2).all(|w| w[1] >= w[0]) && top.last() == Some(&1.0),
            "top band should climb to full strength: {top:?}"
        );
        assert!(
            top[1] < 1.0 && top[2] < 1.0,
            "top band should ease in over {FADE_RAMP}px, not jump: {top:?}"
        );
        assert!(
            bottom[0] <= 0.001,
            "the bottom band should be gone at the end of the document: {bottom:?}"
        );
        assert!(
            bottom.windows(2).all(|w| w[1] >= w[0]) && bottom.last() == Some(&1.0),
            "bottom band should climb away from the end: {bottom:?}"
        );
        assert!(
            bottom[1] < 1.0 && bottom[2] < 1.0,
            "bottom band should ease out over {FADE_RAMP}px, not jump: {bottom:?}"
        );
        // Full strength well inside the scroll range.
        assert_eq!(
            bottom_fade_intensity(100.0, viewport, content, no_caret),
            1.0
        );
        assert_eq!(top_fade_intensity(100.0, viewport, no_caret), 1.0);
    }

    // A band must glide, not switch: a bounded step per frame, the same glide at
    // any frame rate, and it comes to rest exactly on its target.
    #[test]
    fn fade_bands_ease_instead_of_popping() {
        let mut value = 0.0f32;
        let mut from_60fps = Vec::new();
        for _ in 0..6 {
            value = ease_fade(value, 1.0, 1.0 / 60.0);
            from_60fps.push(value);
        }
        assert!(
            from_60fps.windows(2).all(|w| w[1] - w[0] <= 0.30),
            "a band must not jump more than a third of its range in a frame: {from_60fps:?}"
        );
        assert!(
            from_60fps[0] > 0.05,
            "band should start moving at once: {from_60fps:?}"
        );
        let mut slow = 0.0f32;
        for _ in 0..3 {
            slow = ease_fade(slow, 1.0, 1.0 / 30.0);
        }
        assert!(
            (slow - from_60fps[5]).abs() < 0.05,
            "the glide should not depend on the frame rate: 60fps {:.3} vs 30fps {slow:.3}",
            from_60fps[5]
        );
        // Converges exactly, so no band sits at 0.997 forever.
        let mut settled = 0.0f32;
        for _ in 0..200 {
            settled = ease_fade(settled, 1.0, 1.0 / 60.0);
        }
        assert_eq!(settled, 1.0);
    }

    // The caret's line is what the writer is looking at: a band that reaches it
    // goes to zero, whatever the scroll position says about content below.
    #[test]
    fn fade_bands_leave_the_caret_line_alone() {
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let content = 1000.0f32;
        let offset = 300.0f32;
        // The caret's line ends 10px above the bottom edge, inside the band.
        // The caret rect is in window coordinates, as the editor reports it.
        let in_band = egui::Rect::from_min_max(egui::pos2(12.0, 584.0), egui::pos2(600.0, 590.0));
        assert_eq!(
            bottom_fade_intensity(offset, viewport, content, in_band),
            0.0,
            "bottom band covered the caret's line at {:.1}px from the edge",
            viewport.bottom() - in_band.bottom()
        );
        // The caret's line starts 20px under the top edge, inside the header band.
        let under_header =
            egui::Rect::from_min_max(egui::pos2(12.0, 20.0), egui::pos2(600.0, 37.0));
        assert_eq!(top_fade_intensity(offset, viewport, under_header), 0.0);
        // Clear of both bands: full strength.
        let clear = egui::Rect::from_min_max(egui::pos2(12.0, 400.0), egui::pos2(600.0, 417.0));
        assert_eq!(bottom_fade_intensity(offset, viewport, content, clear), 1.0);
        assert_eq!(top_fade_intensity(offset, viewport, clear), 1.0);
    }

    // The scrolling rule, as a function of the caret's row and the viewport.
    #[test]
    fn the_last_row_always_rests_at_the_writing_position() {
        let rest = CaretView {
            caret_top: 600.0,
            caret_bottom: 618.0,
            on_last_row: true,
            view_top: 0.0,
            view_bottom: 650.0,
        };
        // The writing position is 630. A line above it comes down to it, which
        // is what closes the space under a deleted line.
        assert_eq!(caret_scroll(rest), 618.0 - 630.0);
        // A line cut by the bottom edge: the frame Enter adds it.
        let cut = CaretView {
            caret_top: 648.0,
            caret_bottom: 666.0,
            ..rest
        };
        assert_eq!(caret_scroll(cut), 666.0 - 630.0);
        // A line far above the top band, after a block deleted at the end.
        let high = CaretView {
            caret_top: 10.0,
            caret_bottom: 28.0,
            ..rest
        };
        assert_eq!(caret_scroll(high), 28.0 - 630.0);
        // Exactly at the position: the view does not move while typing.
        let typed = CaretView {
            caret_top: 612.0,
            caret_bottom: 630.0,
            ..rest
        };
        assert_eq!(caret_scroll(typed), 0.0);
    }

    #[test]
    fn a_caret_in_the_middle_moves_the_least_that_clears_the_bands() {
        let view = CaretView {
            caret_top: 640.0,
            caret_bottom: 658.0,
            on_last_row: false,
            view_top: 0.0,
            view_bottom: 650.0,
        };
        assert_eq!(caret_scroll(view), 658.0 - (650.0 - CARET_BOTTOM_CLEARANCE));
        let safe = CaretView {
            caret_top: 300.0,
            caret_bottom: 318.0,
            ..view
        };
        assert_eq!(caret_scroll(safe), 0.0);
        let under_header = CaretView {
            caret_top: 12.0,
            caret_bottom: 30.0,
            ..view
        };
        assert_eq!(caret_scroll(under_header), 12.0 - CARET_TOP_CLEARANCE);
    }

    // The space under the text sets how far the view can scroll, so it decides
    // where the last line can rest.
    #[test]
    fn the_trailing_pad_holds_the_view_at_the_writing_position() {
        // Box and text agree: the rest gap under the last row.
        assert_eq!(trailing_pad(100.0, 100.0), CARET_REST_GAP);
        // The pass Enter lands in: the box is a row short of the text, and the
        // gap is measured from the text, so the row the caret is on has room.
        assert_eq!(trailing_pad(600.0, 666.0), 86.0);
        // The pass a Backspace joins two lines in: the box is a row taller than
        // the text, and the space shrinks with the text instead of propping the
        // content open.
        assert_eq!(trailing_pad(618.0, 600.0), 2.0);
        // A short document: the box is the content, so its bottom is the edge.
        assert_eq!(trailing_pad(600.0, 100.0), 0.0);
    }
}
