//! Audited AppKit adapter. The window handle is borrowed from the live eframe
//! frame on its main thread; callers never supply arbitrary native addresses.
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
// The no-window native contract probe is called only by unit tests.
#[allow(unsafe_code, dead_code)]
#[cxx::bridge(namespace = "multicad::window")]
mod ffi {
    struct NativeTouchFrame {
        delta_x: f64,
        delta_y: f64,
        origin_x: f64,
        origin_y: f64,
        sequence: u64,
        active: bool,
        available: bool,
    }
    unsafe extern "C++" {
        include!("native_window.hpp");
        fn position_buttons(view: usize, header_height: f64) -> bool;
        fn install_quit_guard(view: usize) -> u8;
        fn window_identity(view: usize) -> usize;
        fn quit_guard_contract_test() -> bool;
        fn poll_trackpad(view: usize) -> NativeTouchFrame;
        fn trackpad_contract_test() -> bool;
        fn observe_open_documents();
        fn take_opened_documents() -> Vec<String>;
        fn open_documents_contract_test() -> bool;
    }
    extern "Rust" {
        fn wake_for_opened_documents();
        fn observed_quit_shortcut(window: usize, stamp: u64, repeated: bool);
    }
}
fn observed_quit_shortcut(window: usize, stamp: u64, repeated: bool) {
    crate::quit_shortcut::observe(window, stamp, repeated);
}
pub(crate) fn window_identity(frame: &eframe::Frame) -> Option<usize> {
    let handle = frame.window_handle().ok()?;
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return None;
    };
    let id = ffi::window_identity(appkit.ns_view.as_ptr() as usize);
    (id != 0).then_some(id)
}
/// Context to repaint when the operating system asks to open a document while
/// the window is idle. Set from the live frame; never holds native handles.
static OPEN_WAKER: std::sync::OnceLock<eframe::egui::Context> = std::sync::OnceLock::new();
fn wake_for_opened_documents() {
    if let Some(ctx) = OPEN_WAKER.get() {
        ctx.request_repaint();
    }
}
/// Must run on the main thread before the event loop starts launching, so the
/// handler is in place for documents that launch the application.
pub(crate) fn observe_open_documents() {
    ffi::observe_open_documents();
}
// Polled by the live application frame; unit tests use the contract probe.
#[cfg_attr(test, allow(dead_code))]
pub(crate) fn take_opened_documents(ctx: &eframe::egui::Context) -> Vec<std::path::PathBuf> {
    let _ = OPEN_WAKER.set(ctx.clone());
    ffi::take_opened_documents()
        .into_iter()
        .map(std::path::PathBuf::from)
        .collect()
}
fn quit_guard_result(status: u8) -> Result<(), &'static str> {
    match status {
        0 => Ok(()),
        1 => Err("The macOS Quit guard must run on the main thread"),
        2 => Err("The macOS Quit guard has no live application window"),
        3 => Err("The macOS Quit guard has no application delegate"),
        4 => Err(
            "The application delegate already implements Quit handling; its callback was preserved",
        ),
        _ => Err("The macOS Quit guard could not register its termination callback"),
    }
}
pub(crate) fn guard_application_quit(frame: &eframe::Frame) -> Result<(), String> {
    static LOGGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let result = (|| {
        let handle = frame
            .window_handle()
            .map_err(|e| format!("No live macOS window handle: {e}"))?;
        let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
            return Err("The macOS Quit guard requires an AppKit window".into());
        };
        // Borrowed from the live frame, as for title-button alignment. The native
        // adapter retains only weak window state and preserves the winit delegate.
        quit_guard_result(ffi::install_quit_guard(appkit.ns_view.as_ptr() as usize))
            .map_err(str::to_string)
    })();
    if let Err(error) = &result {
        if !LOGGED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            eprintln!("ToskCAD native close protection failed: {error}");
        }
    } else {
        LOGGED.store(false, std::sync::atomic::Ordering::Relaxed);
    }
    result
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
/// Drain every native frame, including welcome/modals, so touches cannot replay
/// into a subsequently opened document. Native coordinates are AppKit points.
pub(crate) fn feed_trackpad(frame: &eframe::Frame, ctx: &eframe::egui::Context) {
    let mut orbit = crate::camera_gestures::NativeOrbit::default();
    if let Ok(handle) = frame.window_handle()
        && let RawWindowHandle::AppKit(appkit) = handle.as_raw()
    {
        let input = ffi::poll_trackpad(appkit.ns_view.as_ptr() as usize);
        if input.available {
            let zoom = ctx.zoom_factor() as f64;
            orbit = crate::camera_gestures::NativeOrbit {
                delta: eframe::egui::vec2(
                    (input.delta_x / zoom) as f32,
                    (input.delta_y / zoom) as f32,
                ),
                origin: eframe::egui::pos2(
                    (input.origin_x / zoom) as f32,
                    (input.origin_y / zoom) as f32,
                ),
                sequence: input.sequence,
                active: input.active,
            };
        }
    }
    crate::camera_gestures::feed_native(ctx, orbit);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quit_guard_preserves_delegate_identity_callbacks_and_foreign_termination() {
        assert!(ffi::quit_guard_contract_test());
    }
    #[test]
    fn trackpad_observer_preserves_host_callbacks_and_tracks_exactly_three_contacts() {
        assert!(ffi::trackpad_contract_test());
    }
    #[test]
    fn finder_open_documents_events_queue_only_file_paths_in_order() {
        assert!(ffi::open_documents_contract_test());
    }
    #[test]
    fn quit_guard_failures_never_report_protection_as_installed() {
        assert!(quit_guard_result(0).is_ok());
        for status in [1, 2, 3, 4, 5, u8::MAX] {
            assert!(quit_guard_result(status).is_err());
        }
        assert!(quit_guard_result(4).unwrap_err().contains("preserved"));
    }
}
