//! Talking to the copy of Voiceolator that is already running:
//!
//! * starting Voiceolator again while it is hidden in the tray brings the running copy
//!   back instead of opening a second one;
//! * files given to the second start (dropped on the program icon, "Isolate the voice
//!   with Voiceolator" in the file manager) are handed over to its Files tab;
//! * `Voiceolator --quit` asks it to quit (used by the installer).
//!
//! Windows: a named event the running copy waits on, plus small message files in
//! `%LOCALAPPDATA%\Voiceolator\inbox`. Linux: a Unix socket in `$XDG_RUNTIME_DIR`.

use crate::app::UiCommand;
use eframe::egui;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

/// Message lines: `show`, `file:<path>`, `quit`.
fn encode(files: &[PathBuf], quit: bool) -> String {
    let mut m = String::from(if quit { "quit\n" } else { "show\n" });
    for f in files {
        m.push_str(&format!("file:{}\n", f.display()));
    }
    m
}

fn decode(msg: &str, tx: &Sender<UiCommand>) -> bool {
    let mut files = Vec::new();
    let mut quit = false;
    for l in msg.lines() {
        if l == "quit" {
            quit = true;
        } else if let Some(p) = l.strip_prefix("file:") {
            files.push(PathBuf::from(p));
        }
    }
    let cmd = if quit {
        UiCommand::Quit
    } else if files.is_empty() {
        UiCommand::ShowWindow
    } else {
        UiCommand::AddFiles(files)
    };
    tx.send(cmd).is_ok()
}

/// Retry for a few seconds: several copies may start at once (multi-select "open with"),
/// and the first one needs a moment before it listens.
#[allow(dead_code)]
fn retry(mut f: impl FnMut() -> bool) -> bool {
    let t = Instant::now();
    loop {
        if f() {
            return true;
        }
        if t.elapsed() > Duration::from_secs(5) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use windows::core::w;
    use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{CreateEventW, OpenEventW, SetEvent, WaitForSingleObject, EVENT_MODIFY_STATE, INFINITE};

    fn inbox() -> PathBuf {
        crate::config::log_path().with_file_name("inbox")
    }

    pub fn listen(ctx: egui::Context, tx: Sender<UiCommand>) {
        let Ok(ev) = (unsafe { CreateEventW(None, false, false, w!("Local\\VoiceolatorShowWindow")) }) else { return };
        let ev = ev.0 as isize;
        let _ = std::thread::Builder::new().name("vo-ipc".into()).spawn(move || loop {
            if unsafe { WaitForSingleObject(HANDLE(ev as *mut core::ffi::c_void), INFINITE) } != WAIT_OBJECT_0 {
                return;
            }
            let mut msgs: Vec<PathBuf> = std::fs::read_dir(inbox()).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default();
            msgs.sort();
            let mut any = false;
            for m in msgs {
                let text = std::fs::read_to_string(&m).unwrap_or_default();
                let _ = std::fs::remove_file(&m);
                if !text.starts_with("quit") {
                    crate::tray::show_native();
                }
                any = true;
                if !decode(&text, &tx) {
                    return;
                }
            }
            if !any {
                crate::tray::show_native();
                let _ = tx.send(UiCommand::ShowWindow);
            }
            ctx.request_repaint();
        });
    }

    fn send(msg: String) -> bool {
        let dir = inbox();
        let _ = std::fs::create_dir_all(&dir);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let f = dir.join(format!("{nanos}-{}.txt", std::process::id()));
        if std::fs::write(&f, msg).is_err() {
            return false;
        }
        let ok = retry(|| unsafe {
            match OpenEventW(EVENT_MODIFY_STATE, false, w!("Local\\VoiceolatorShowWindow")) {
                Ok(h) => {
                    let ok = SetEvent(h).is_ok();
                    let _ = CloseHandle(h);
                    ok
                }
                Err(_) => false,
            }
        });
        if !ok {
            let _ = std::fs::remove_file(&f);
        }
        ok
    }

    pub fn notify_running(files: &[PathBuf]) -> bool {
        send(encode(files, false))
    }

    pub fn send_quit() -> bool {
        send(encode(&[], true))
    }
}

#[cfg(unix)]
mod imp {
    use super::*;
    use std::io::{Read, Write};
    use std::os::unix::net::{UnixListener, UnixStream};

    fn path() -> PathBuf {
        std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("voiceolator.sock")
    }

    /// Call only while holding the single-instance lock.
    pub fn listen(ctx: egui::Context, tx: Sender<UiCommand>) {
        let p = path();
        let _ = std::fs::remove_file(&p);
        let Ok(l) = UnixListener::bind(&p) else { return };
        let _ = std::thread::Builder::new().name("vo-ipc".into()).spawn(move || {
            for s in l.incoming().flatten() {
                let mut msg = String::new();
                let _ = s.take(1 << 20).read_to_string(&mut msg);
                if !decode(&msg, &tx) {
                    return;
                }
                ctx.request_repaint();
            }
        });
    }

    fn send(msg: String) -> bool {
        retry(|| match UnixStream::connect(path()) {
            Ok(mut s) => s.write_all(msg.as_bytes()).is_ok(),
            Err(_) => false,
        })
    }

    pub fn notify_running(files: &[PathBuf]) -> bool {
        send(encode(files, false))
    }

    pub fn send_quit() -> bool {
        send(encode(&[], true))
    }
}

#[cfg(not(any(windows, unix)))]
mod imp {
    use super::*;
    pub fn listen(_ctx: egui::Context, _tx: Sender<UiCommand>) {}
    pub fn notify_running(_files: &[PathBuf]) -> bool {
        false
    }
    pub fn send_quit() -> bool {
        false
    }
}

pub use imp::{listen, notify_running, send_quit};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_roundtrip() {
        let (tx, rx) = std::sync::mpsc::channel();
        assert!(decode(&encode(&[], false), &tx));
        assert_eq!(rx.recv().unwrap(), UiCommand::ShowWindow);
        let files = vec![PathBuf::from("/a b/vod.mp4"), PathBuf::from("c.mp3")];
        assert!(decode(&encode(&files, false), &tx));
        assert_eq!(rx.recv().unwrap(), UiCommand::AddFiles(files));
        assert!(decode(&encode(&[], true), &tx));
        assert_eq!(rx.recv().unwrap(), UiCommand::Quit);
    }
}
