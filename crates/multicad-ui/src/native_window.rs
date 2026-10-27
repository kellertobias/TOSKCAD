//! Audited AppKit adapter. The window handle is borrowed from the live eframe
//! frame on its main thread; callers never supply arbitrary native addresses.
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
// The no-window native contract probe is called only by unit tests.
#[allow(unsafe_code, dead_code)]
#[cxx::bridge(namespace = "multicad::window")]
mod ffi {
    unsafe extern "C++" {
        include!("native_window.hpp");
        fn position_buttons(view: usize, header_height: f64) -> bool;
        fn install_quit_guard(view: usize) -> u8;
        fn quit_guard_contract_test() -> bool;
    }
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
            eprintln!("MultiCAD native close protection failed: {error}");
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
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quit_guard_preserves_delegate_identity_callbacks_and_foreign_termination() {
        assert!(ffi::quit_guard_contract_test());
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
