//! System-wide hotkeys, so the app can be controlled without leaving the game:
//! Ctrl+Alt+V toggles voice isolation, Ctrl+Alt+M mutes Voiceolator's output.

use crate::app::UiCommand;
use std::sync::mpsc::Sender;

#[cfg(windows)]
pub fn start(ctx: eframe::egui::Context, tx: Sender<UiCommand>) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT};
    use windows::Win32::UI::WindowsAndMessaging::{GetMessageW, MSG, WM_HOTKEY};
    let _ = std::thread::Builder::new()
        .name("vo-hotkeys".into())
        .spawn(move || unsafe {
            let mods = MOD_CONTROL | MOD_ALT | MOD_NOREPEAT;
            let a = RegisterHotKey(None, 1, mods, 'V' as u32).is_ok();
            let b = RegisterHotKey(None, 2, mods, 'M' as u32).is_ok();
            if !a || !b {
                log::warn!("some global hotkeys could not be registered (already used by another app?)");
            }
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                if msg.message == WM_HOTKEY {
                    let cmd = if msg.wParam.0 == 1 { UiCommand::ToggleIsolation } else { UiCommand::ToggleMute };
                    if tx.send(cmd).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            }
        });
}

#[cfg(not(windows))]
pub fn start(_ctx: eframe::egui::Context, _tx: Sender<UiCommand>) {}
