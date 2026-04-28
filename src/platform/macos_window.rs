//! Manual NSWindow traffic-light positioning. With
//! `setTitlebarAppearsTransparent + fullsize_content_view` the OS keeps the
//! close/minimize/zoom buttons at their default y (~14pt from window top).
//! Our title bar is taller than the OS default, so `items-center` puts the
//! bar's content well below those buttons. We call `setFrameOrigin:` on each
//! standard window button to move them to the y that aligns with our bar's
//! content centerline.

use objc2::rc::Retained;
use objc2_app_kit::{NSWindow, NSWindowButton};
use objc2_foundation::NSPoint;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

/// Reposition the window's traffic lights so their vertical centers land at
/// `target_center_y_from_top` (logical points, measured from the window top).
/// `left_margin` is the leftmost button's x in logical points.
///
/// AppKit views use a bottom-left origin in their parent, so the conversion
/// is `origin_y_from_bottom = titlebar_height - target_top - button_height`.
/// We reuse the OS-decided horizontal spacing between buttons.
pub fn position_traffic_lights(window: &Window, left_margin: f32, target_center_y_from_top: f32) {
    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return;
    };

    unsafe {
        let ns_view: *mut objc2::runtime::AnyObject = handle.ns_view.as_ptr().cast();
        let ns_view: &objc2::runtime::AnyObject = &*ns_view;
        let ns_window: Option<Retained<NSWindow>> =
            objc2::msg_send![ns_view, window];
        let Some(ns_window) = ns_window else {
            return;
        };

        let close = ns_window.standardWindowButton(NSWindowButton::CloseButton);
        let minimize = ns_window.standardWindowButton(NSWindowButton::MiniaturizeButton);
        let zoom = ns_window.standardWindowButton(NSWindowButton::ZoomButton);
        let (Some(close), Some(minimize), Some(zoom)) = (close, minimize, zoom) else {
            return;
        };

        let close_frame = close.frame();
        let min_frame = minimize.frame();
        let button_h = close_frame.size.height as f32;
        let button_spacing = (min_frame.origin.x - close_frame.origin.x) as f32;

        // AppKit's window content area uses bottom-left origin, but
        // `setFrameOrigin:` on a standard window button measures from the
        // titlebar's bottom (the buttons live inside the titlebar's frame
        // view). Hence `y_from_bottom = titlebar_h - top_y - button_h`.
        let titlebar_h = titlebar_height(&ns_window);
        let target_top = target_center_y_from_top - button_h * 0.5;
        let y = (titlebar_h - target_top - button_h) as f64;

        let mut x = left_margin as f64;
        close.setFrameOrigin(NSPoint::new(x, y));
        x += button_spacing as f64;
        minimize.setFrameOrigin(NSPoint::new(x, y));
        x += button_spacing as f64;
        zoom.setFrameOrigin(NSPoint::new(x, y));
    }
}

fn titlebar_height(ns_window: &NSWindow) -> f32 {
    unsafe {
        let frame = ns_window.frame();
        let content_layout: objc2_foundation::NSRect =
            objc2::msg_send![ns_window, contentLayoutRect];
        (frame.size.height - content_layout.size.height) as f32
    }
}
