//! Round the borderless Linux window, including anything drawn over its corners.

use std::sync::Arc;

use eframe::{egui, egui_glow, glow, glow::HasContext};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use x11rb::{
    protocol::{
        shape::{ConnectionExt as _, SK, SO},
        xproto::{ClipOrdering, Rectangle, Window},
    },
    rust_connection::RustConnection,
    NONE,
};

const CORNER_RADIUS: f32 = 10.0;

#[derive(Default)]
pub(crate) struct WindowCorners {
    x11: Option<X11Window>,
}

struct X11Window {
    connection: RustConnection,
    id: Window,
    shape: Option<(i16, i16, i16)>,
}

impl WindowCorners {
    pub(crate) fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let x11 = (|| {
            let id = match cc.window_handle().ok()?.as_raw() {
                RawWindowHandle::Xlib(handle) => Window::try_from(handle.window).ok()?,
                RawWindowHandle::Xcb(handle) => handle.window.get(),
                _ => return None,
            };
            let (connection, _) = x11rb::connect(None).ok()?;
            connection.shape_query_version().ok()?.reply().ok()?;
            Some(X11Window {
                connection,
                id,
                shape: None,
            })
        })();
        Self { x11 }
    }

    pub(crate) fn update(&mut self, ctx: &egui::Context) {
        let square = ctx.input(|input| {
            let viewport = input.viewport();
            viewport.maximized == Some(true) || viewport.fullscreen == Some(true)
        });
        let screen = ctx.screen_rect();
        let scale = ctx.pixels_per_point();
        let radius = if square {
            0
        } else {
            (CORNER_RADIUS * scale).round() as i16
        };

        // X11's bounding shape also clips the input region and works without a compositor.
        if let Some(window) = &mut self.x11 {
            let width = (screen.width() * scale).round() as i16;
            let height = (screen.height() * scale).round() as i16;
            if window.set_shape(width, height, radius) {
                return;
            }
            self.x11 = None;
        }

        if radius == 0 {
            return;
        }
        // Wayland composites the alpha buffer. Clear after all egui layers so
        // scrollbar, caption hover, and modal paint share the same outline.
        ctx.debug_painter().add(egui::PaintCallback {
            rect: screen,
            callback: Arc::new(egui_glow::CallbackFn::new(move |info, painter| {
                let [width, height] = info.screen_size_px.map(|value| value as i32);
                let gl = painter.gl();
                // The callback runs on egui's current framebuffer with scissoring enabled.
                unsafe {
                    gl.clear_color(0.0, 0.0, 0.0, 0.0);
                    for row in 0..radius {
                        let inset = i32::from(corner_inset(radius, row));
                        if inset > 0 {
                            for y in [i32::from(row), height - 1 - i32::from(row)] {
                                gl.scissor(0, y, inset, 1);
                                gl.clear(glow::COLOR_BUFFER_BIT);
                                gl.scissor(width - inset, y, inset, 1);
                                gl.clear(glow::COLOR_BUFFER_BIT);
                            }
                        }
                    }
                }
            })),
        });
    }
}

impl X11Window {
    fn set_shape(&mut self, width: i16, height: i16, radius: i16) -> bool {
        if width <= 2 * radius || height <= 2 * radius {
            return true;
        }
        let shape = (width, height, radius);
        if self.shape == Some(shape) {
            return true;
        }
        let result = if radius == 0 {
            self.connection
                .shape_mask(SO::SET, SK::BOUNDING, self.id, 0, 0, NONE)
        } else {
            let mut rows = vec![Rectangle {
                x: 0,
                y: radius,
                width: width as u16,
                height: (height - 2 * radius) as u16,
            }];
            for row in 0..radius {
                let inset = corner_inset(radius, row);
                for y in [row, height - 1 - row] {
                    rows.push(Rectangle {
                        x: inset,
                        y,
                        width: (width - 2 * inset) as u16,
                        height: 1,
                    });
                }
            }
            self.connection.shape_rectangles(
                SO::SET,
                SK::BOUNDING,
                ClipOrdering::UNSORTED,
                self.id,
                0,
                0,
                &rows,
            )
        };
        if result.is_ok_and(|cookie| cookie.check().is_ok()) {
            self.shape = Some(shape);
            true
        } else {
            false
        }
    }
}

// Measure the circle at pixel centers so the two sides of each corner match.
fn corner_inset(radius: i16, row: i16) -> i16 {
    let radius = f32::from(radius);
    let y = radius - f32::from(row) - 0.5;
    (radius - (radius * radius - y * y).sqrt()).round() as i16
}
