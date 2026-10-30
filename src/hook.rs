use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    EVENT_OBJECT_LOCATIONCHANGE, EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZEEND,
    EVENT_SYSTEM_MOVESIZESTART, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
};

pub struct EventHookHandle {
    hooks: Vec<HWINEVENTHOOK>,
}

impl Drop for EventHookHandle {
    fn drop(&mut self) {
        for hook in &self.hooks {
            if !hook.is_invalid() {
                unsafe {
                    let _ = UnhookWinEvent(*hook);
                }
            }
        }
    }
}

pub fn install_window_event_hook(
    proc: unsafe extern "system" fn(HWINEVENTHOOK, u32, HWND, i32, i32, u32, u32),
) -> Option<EventHookHandle> {
    unsafe {
        let mut hooks = Vec::new();

        // 1. Hook window move/resize/minimize/restore events (0x000A to 0x0017)
        let h1 = SetWinEventHook(
            EVENT_SYSTEM_MOVESIZESTART,
            EVENT_SYSTEM_MINIMIZEEND,
            None,
            Some(proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        );
        if !h1.is_invalid() {
            hooks.push(h1);
        }

        // 2. Hook foreground active window change (0x0003)
        let h2 = SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        );
        if !h2.is_invalid() {
            hooks.push(h2);
        }

        // 3. Hook object location change for window bounds settling (0x800B)
        let h3 = SetWinEventHook(
            EVENT_OBJECT_LOCATIONCHANGE,
            EVENT_OBJECT_LOCATIONCHANGE,
            None,
            Some(proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        );
        if !h3.is_invalid() {
            hooks.push(h3);
        }

        if hooks.is_empty() {
            None
        } else {
            Some(EventHookHandle { hooks })
        }
    }
}
