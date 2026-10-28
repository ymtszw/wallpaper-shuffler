use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    EVENT_OBJECT_LOCATIONCHANGE, EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MOVESIZEEND,
    WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
};

pub struct EventHookHandle {
    hook: HWINEVENTHOOK,
}

impl Drop for EventHookHandle {
    fn drop(&mut self) {
        if !self.hook.is_invalid() {
            unsafe {
                let _ = UnhookWinEvent(self.hook);
            }
        }
    }
}

pub type WinEventCallback = Box<dyn FnMut(u32, HWND) + Send + 'static>;

pub fn install_window_event_hook(
    proc: unsafe extern "system" fn(HWINEVENTHOOK, u32, HWND, i32, i32, u32, u32),
) -> Option<EventHookHandle> {
    unsafe {
        let hook = SetWinEventHook(
            EVENT_SYSTEM_MOVESIZEEND,
            EVENT_OBJECT_LOCATIONCHANGE,
            None,
            Some(proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        );
        if hook.is_invalid() {
            None
        } else {
            Some(EventHookHandle { hook })
        }
    }
}

pub const MONITORED_EVENTS: [u32; 3] = [
    EVENT_SYSTEM_MOVESIZEEND,
    EVENT_SYSTEM_MINIMIZESTART,
    EVENT_OBJECT_LOCATIONCHANGE,
];
