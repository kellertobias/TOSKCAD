//! Native quit key evidence is separate from window close notifications.
//! Duplicate callbacks and held-key repeat never authorize discarding a file.
use std::{collections::VecDeque, sync::Mutex};

#[derive(Default)]
struct PressQueue {
    last: VecDeque<(usize, u64)>,
    pending: VecDeque<usize>,
}
impl PressQueue {
    #[cfg_attr(
        not(any(test, target_os = "macos", target_os = "windows")),
        allow(dead_code)
    )]
    fn observe(&mut self, window: usize, stamp: u64, repeated: bool) {
        if window == 0 || repeated || self.last.contains(&(window, stamp)) {
            return;
        }
        self.last.retain(|(id, _)| *id != window);
        self.last.push_back((window, stamp));
        self.pending.push_back(window);
        while self.last.len() > 32 {
            self.last.pop_front();
        }
        while self.pending.len() > 32 {
            self.pending.pop_front();
        }
    }
    fn take(&mut self, window: usize) -> usize {
        let count = self.pending.iter().filter(|&&id| id == window).count();
        self.pending.retain(|&id| id != window);
        count
    }
}
static PRESSES: Mutex<PressQueue> = Mutex::new(PressQueue {
    last: VecDeque::new(),
    pending: VecDeque::new(),
});
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(crate) fn observe(window: usize, stamp: u64, repeated: bool) {
    if let Ok(mut queue) = PRESSES.lock() {
        queue.observe(window, stamp, repeated);
    }
}
pub(crate) fn feed(frame: &eframe::Frame, ctx: &crate::egui::Context) {
    #[cfg(target_os = "macos")]
    let window = crate::native_window::window_identity(frame);
    #[cfg(target_os = "windows")]
    let window = windows::window_identity(frame);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let window = {
        let _ = frame;
        None
    };
    let count = window
        .and_then(|id| PRESSES.lock().ok().map(|mut q| q.take(id)))
        .unwrap_or(0);
    ctx.data_mut(|data| data.insert_temp(crate::egui::Id::new("native-quit-presses"), count));
}
pub(crate) fn take(ctx: &crate::egui::Context) -> usize {
    let native = ctx
        .data_mut(|data| data.remove_temp::<usize>(crate::egui::Id::new("native-quit-presses")))
        .unwrap_or(0);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let native = native
        + ctx.input_mut(|input| {
            let mut presses = 0;
            input.events.retain(|event| {
                if let crate::egui::Event::Key {
                    key: crate::egui::Key::Q,
                    pressed: true,
                    repeat,
                    modifiers,
                    ..
                } = event
                    && modifiers.ctrl
                    && !modifiers.alt
                    && !modifiers.shift
                {
                    presses += usize::from(!repeat);
                    false
                } else {
                    true
                }
            });
            presses
        });
    native
}

#[cfg(target_os = "windows")]
pub(crate) mod windows {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::UI::WindowsAndMessaging::{MSG, WM_SYSKEYDOWN};
    use winit::platform::windows::EventLoopBuilderExtWindows;

    pub(crate) fn configure(options: &mut eframe::NativeOptions) {
        options.event_loop_builder = Some(Box::new(|builder| {
            builder.with_msg_hook(observe_message);
        }));
    }
    // Winit supplies a live MSG only during this callback. It is borrowed,
    // read once, never retained, and always dispatched through the host.
    #[allow(unsafe_code)]
    fn observe_message(message: *const std::ffi::c_void) -> bool {
        if !message.is_null() {
            let message = unsafe { &*message.cast::<MSG>() };
            if message.message == WM_SYSKEYDOWN
                && message.wParam == 0x73 // VK_F4
                && message.lParam & (1 << 29) != 0
            // Alt context
            {
                super::observe(
                    message.hwnd as usize,
                    u64::from(message.time),
                    message.lParam & (1 << 30) != 0,
                );
            }
        }
        false
    }
    pub(super) fn window_identity(frame: &eframe::Frame) -> Option<usize> {
        match frame.window_handle().ok()?.as_raw() {
            RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as usize),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_native_delivery_and_auto_repeat_are_not_new_presses() {
        let mut q = PressQueue::default();
        q.observe(1, 10, false);
        q.observe(1, 10, false);
        q.observe(1, 11, true);
        assert_eq!(q.take(1), 1);
        q.observe(1, 12, false);
        assert_eq!(q.take(1), 1);
        assert_eq!(q.take(1), 0);
    }
    #[test]
    fn press_evidence_is_window_scoped_and_bounded() {
        let mut q = PressQueue::default();
        for n in 1..=100 {
            q.observe(n, 0, false);
        }
        assert_eq!(q.take(1), 0);
        assert_eq!(q.take(99), 1);
        assert_eq!(q.take(100), 1);
        assert!(q.last.len() <= 32 && q.pending.len() <= 32);
        q.observe(0, 100, false);
        assert_eq!(q.take(0), 0);
    }
}
