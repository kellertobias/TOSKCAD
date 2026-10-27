//! Audited AppKit adapter. The window handle is borrowed from the live eframe
//! frame on its main thread; callers never supply arbitrary native addresses.
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
#[allow(unsafe_code)]
#[cxx::bridge(namespace = "multicad::window")]
mod ffi {
    unsafe extern "C++" {
        include!("native_window.hpp");
        fn position_buttons(view: usize, header_height: f64) -> bool;
    }
}
pub(crate) fn align_title_buttons(frame: &eframe::Frame, height: f64) {
    if !height.is_finite() || !(28.0..=80.0).contains(&height) {
        return;
    }
    if let Ok(handle) = frame.window_handle()
        && let RawWindowHandle::AppKit(appkit) = handle.as_raw()
    {
        // WindowHandle's lifetime guarantees this NSView belongs to the live
        // window. AppKit independently rejects calls outside the main thread.
        ffi::position_buttons(appkit.ns_view.as_ptr() as usize, height);
    }
}
