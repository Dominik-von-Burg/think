//! Paint GTK title buttons inside Think's custom title bar.

use std::ffi::{c_char, c_double, c_int, c_void, CStr};
use std::ptr;
use std::sync::OnceLock;

use eframe::egui;
use libloading::Library;

use crate::title_bar::CaptionButton;

const SCALE: i32 = 2;

pub(crate) struct LinuxCaption {
    // Minimize, maximize, restore, close. Each has normal, hover, and pressed textures.
    buttons: [[egui::TextureHandle; 3]; 4],
}

impl LinuxCaption {
    pub(crate) fn new(ctx: &egui::Context) -> Option<Self> {
        // GTK is optional so the app can still launch on Linux systems without it.
        // GTK and the offscreen widgets are used only on the UI thread.
        // Keep GTK loaded because it registers callbacks with GLib.
        static GTK: OnceLock<Option<Library>> = OnceLock::new();
        let gtk = GTK.get_or_init(|| unsafe { Library::new("libgtk-3.so.0").ok() });
        unsafe { Self::render(ctx, gtk.as_ref()?) }
    }

    pub(crate) fn button(
        &self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        kind: CaptionButton,
    ) -> egui::Response {
        let response = ui.allocate_rect(rect, egui::Sense::click());
        let button = match kind {
            CaptionButton::Minimize => 0,
            CaptionButton::Maximize => 1,
            CaptionButton::Restore => 2,
            CaptionButton::Close => 3,
        };
        let state = if response.is_pointer_button_down_on() {
            2
        } else {
            usize::from(response.hovered())
        };
        let texture = &self.buttons[button][state];
        let natural = texture.size_vec2() / SCALE as f32;
        let fit = (rect.width() / natural.x)
            .min(rect.height() / natural.y)
            .min(1.0);
        ui.painter().image(
            texture.id(),
            egui::Rect::from_center_size(rect.center(), natural * fit),
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
        response
    }

    unsafe fn render(ctx: &egui::Context, gtk: &Library) -> Option<Self> {
        type Object = *mut c_void;
        macro_rules! load {
            ($(fn $name:ident($($arg:ty),*) $(-> $ret:ty)?;)*) => {
                $(let $name = gtk.get::<unsafe extern "C" fn($($arg),*) $(-> $ret)?>(
                    concat!(stringify!($name), "\0").as_bytes(),
                ).ok()?;)*
            };
        }
        load! {
            fn gtk_init_check(Object, Object) -> c_int;
            fn gtk_settings_get_default() -> Object;
            fn gtk_offscreen_window_new() -> Object;
            fn gtk_header_bar_new() -> Object;
            fn gtk_container_add(Object, Object);
            fn gtk_button_new() -> Object;
            fn gtk_widget_set_valign(Object, c_int);
            fn gtk_widget_get_style_context(Object) -> Object;
            fn gtk_style_context_add_class(Object, *const c_char);
            fn gtk_image_new_from_icon_name(*const c_char, c_int) -> Object;
            fn gtk_button_set_image(Object, Object);
            fn gtk_header_bar_pack_end(Object, Object);
            fn gtk_widget_show_all(Object);
            fn gtk_events_pending() -> c_int;
            fn gtk_main_iteration_do(c_int) -> c_int;
            fn gtk_widget_get_allocated_width(Object) -> c_int;
            fn gtk_widget_get_allocated_height(Object) -> c_int;
            fn gtk_widget_set_state_flags(Object, c_int, c_int);
            fn gtk_widget_draw(Object, Object);
            fn gtk_widget_destroy(Object);
            fn cairo_image_surface_create(c_int, c_int, c_int) -> Object;
            fn cairo_image_surface_get_data(Object) -> *const u8;
            fn cairo_image_surface_get_stride(Object) -> c_int;
            fn cairo_surface_flush(Object);
            fn cairo_surface_destroy(Object);
            fn cairo_create(Object) -> Object;
            fn cairo_scale(Object, c_double, c_double);
            fn cairo_destroy(Object);
        }
        let set_property = gtk
            .get::<unsafe extern "C" fn(Object, *const c_char, ...)>(b"g_object_set\0")
            .ok()?;

        // GTK constructors, draw functions, and Cairo use borrowed C pointers here.
        // All of the widgets belong to the offscreen window and are destroyed together.
        if gtk_init_check(ptr::null_mut(), ptr::null_mut()) == 0 {
            return None;
        }
        let settings = gtk_settings_get_default();
        let end = ptr::null::<c_char>();
        set_property(settings, c"gtk-enable-animations".as_ptr(), 0 as c_int, end);
        set_property(
            settings,
            c"gtk-application-prefer-dark-theme".as_ptr(),
            1 as c_int,
            end,
        );

        let window = gtk_offscreen_window_new();
        let header = gtk_header_bar_new();
        let add_class = |widget, class: &CStr| {
            gtk_style_context_add_class(gtk_widget_get_style_context(widget), class.as_ptr());
        };
        add_class(header, c"titlebar");
        gtk_container_add(window, header);
        let controls = [
            (c"minimize", c"window-minimize-symbolic"),
            (c"maximize", c"window-maximize-symbolic"),
            (c"maximize", c"window-restore-symbolic"),
            (c"close", c"window-close-symbolic"),
        ];
        let widgets = controls.map(|(class, icon)| {
            let button = gtk_button_new();
            gtk_widget_set_valign(button, 3); // GTK_ALIGN_CENTER
            add_class(button, c"titlebutton");
            add_class(button, class);
            gtk_button_set_image(button, gtk_image_new_from_icon_name(icon.as_ptr(), 1)); // GTK_ICON_SIZE_MENU
            gtk_header_bar_pack_end(header, button);
            button
        });
        gtk_widget_show_all(window);
        for _ in 0..32 {
            if gtk_events_pending() == 0 {
                break;
            }
            gtk_main_iteration_do(0);
        }

        let snapshot = |widget| {
            let width = gtk_widget_get_allocated_width(widget) * SCALE;
            let height = gtk_widget_get_allocated_height(widget) * SCALE;
            if !(2..=256).contains(&width) || !(2..=256).contains(&height) {
                return None;
            }
            let surface = cairo_image_surface_create(0, width, height); // CAIRO_FORMAT_ARGB32
            let drawing = cairo_create(surface);
            cairo_scale(drawing, f64::from(SCALE), f64::from(SCALE));
            gtk_widget_draw(widget, drawing);
            cairo_destroy(drawing);
            cairo_surface_flush(surface);

            let data = cairo_image_surface_get_data(surface);
            let image = (!data.is_null()).then(|| {
                let stride = cairo_image_surface_get_stride(surface) as usize;
                let mut image = egui::ColorImage::new(
                    [width as usize, height as usize],
                    egui::Color32::TRANSPARENT,
                );
                for (i, pixel) in image.pixels.iter_mut().enumerate() {
                    let offset = i / width as usize * stride + i % width as usize * 4;
                    let [a, r, g, b] = data
                        .add(offset)
                        .cast::<u32>()
                        .read_unaligned()
                        .to_be_bytes();
                    *pixel = egui::Color32::from_rgba_premultiplied(r, g, b, a);
                }
                image
            });
            cairo_surface_destroy(surface);
            image
        };

        let buttons = (|| {
            let mut buttons = Vec::new();
            for (index, button) in widgets.into_iter().enumerate() {
                let mut states = Vec::new();
                for state in [0, 2, 3] {
                    // NORMAL, PRELIGHT, PRELIGHT | ACTIVE
                    gtk_widget_set_state_flags(button, state, 1);
                    states.push(ctx.load_texture(
                        format!("gtk-caption-{index}-{state}"),
                        snapshot(button)?,
                        egui::TextureOptions::LINEAR,
                    ));
                }
                buttons.push(states.try_into().ok()?);
            }
            buttons.try_into().ok()
        })();
        gtk_widget_destroy(window);
        Some(Self { buttons: buttons? })
    }
}
