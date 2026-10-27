//! Voiceolator desktop app.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod config;
mod files;
mod hotkeys;
mod ipc;
mod platform;
mod theme;
mod tray;

use std::io::Write;
use std::sync::Mutex;

struct FileLogger {
    file: Mutex<Option<std::fs::File>>,
}

impl log::Log for FileLogger {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Info
    }
    fn log(&self, r: &log::Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        if let Some(f) = self.file.lock().unwrap().as_mut() {
            let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            let _ = writeln!(f, "{t} {:<5} {}: {}", r.level(), r.target(), r.args());
        }
    }
    fn flush(&self) {}
}

fn init_logging() {
    let path = config::log_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::metadata(&path).map(|m| m.len() > 2_000_000).unwrap_or(false) {
        let _ = std::fs::rename(&path, path.with_extension("old.log"));
    }
    let file = std::fs::OpenOptions::new().create(true).append(true).open(&path).ok();
    let logger = Box::leak(Box::new(FileLogger { file: Mutex::new(file) }));
    let _ = log::set_logger(logger).map(|()| log::set_max_level(log::LevelFilter::Info));
    std::panic::set_hook(Box::new(|info| {
        log::error!("panic: {info}");
    }));
    log::info!("Voiceolator {} starting", env!("CARGO_PKG_VERSION"));
}

/// Returns false if another instance is already running (and brings it to the front).
#[cfg(windows)]
fn single_instance(files: &[std::path::PathBuf]) -> bool {
    use windows::core::w;
    use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, SetForegroundWindow, ShowWindow, SW_RESTORE};
    unsafe {
        let h = CreateMutexW(None, true, w!("Local\\VoiceolatorSingleInstance"));
        if h.is_ok() && GetLastError() == ERROR_ALREADY_EXISTS {
            // ask the running copy to show itself (it may be hidden in the tray)
            if ipc::notify_running(files) {
                return false;
            }
            if let Ok(hwnd) = FindWindowW(None, w!("Voiceolator")) {
                let _ = ShowWindow(hwnd, SW_RESTORE);
                let _ = SetForegroundWindow(hwnd);
            }
            return false;
        }
        std::mem::forget(h); // keep the mutex for the process lifetime
    }
    true
}

/// Linux: an advisory lock in the runtime directory (released automatically on exit/crash).
#[cfg(unix)]
fn single_instance(files: &[std::path::PathBuf]) -> bool {
    use std::os::unix::io::AsRawFd;
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let Ok(f) = std::fs::OpenOptions::new().create(true).write(true).truncate(false).open(dir.join("voiceolator.lock")) else { return true };
    let ok = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0;
    if ok {
        std::mem::forget(f); // hold the lock for the process lifetime
    } else if ipc::notify_running(files) {
        eprintln!("Voiceolator is already running; showing its window.");
    } else {
        eprintln!("Voiceolator is already running.");
    }
    ok
}

#[cfg(not(any(windows, unix)))]
fn single_instance(_files: &[std::path::PathBuf]) -> bool {
    true
}

fn load_icon() -> Option<eframe::egui::IconData> {
    let bytes = include_bytes!("../assets/icon.png");
    let dec = png::Decoder::new(std::io::Cursor::new(&bytes[..]));
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    if info.color_type != png::ColorType::Rgba {
        return None;
    }
    buf.truncate(info.buffer_size());
    Some(eframe::egui::IconData { rgba: buf, width: info.width, height: info.height })
}

fn main() {
    vo_audio::watchdog::handle_args();
    let args: Vec<String> = std::env::args().collect();
    // Helper mode for the latency measurement (see vo_audio::windows::latency).
    if args.get(1).map(|a| a == "--beacon").unwrap_or(false) {
        let secs = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(8.0);
        let code = match platform::run_beacon(secs) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("beacon: {e:#}");
                1
            }
        };
        std::process::exit(code);
    }
    // `Voiceolator --quit`: ask a running copy to quit (used by the installer).
    if args.get(1).map(|a| a == "--quit").unwrap_or(false) {
        std::process::exit(if ipc::send_quit() { 0 } else { 1 });
    }
    init_logging();
    // Files passed on the command line (or dropped on the program icon) go to the Files tab.
    let files: Vec<std::path::PathBuf> = args.iter().skip(1).map(std::path::PathBuf::from).filter(|p| p.exists()).collect();
    if !single_instance(&files) {
        return;
    }
    let cfg = config::Config::load();
    let mut viewport = eframe::egui::ViewportBuilder::default().with_title("Voiceolator");
    // open directly in the saved layout (no flash of the full window)
    viewport = match cfg.view {
        config::ViewMode::Full => viewport.with_inner_size(cfg.full_size.unwrap_or(app::FULL_SIZE)).with_min_inner_size(app::FULL_MIN),
        v => {
            let (size, pos) = if v == config::ViewMode::Slim { (app::SLIM_SIZE, cfg.slim_pos) } else { (app::PILL_SIZE, cfg.pill_pos) };
            let mut b = viewport.with_inner_size(size).with_min_inner_size(size).with_decorations(false).with_resizable(false);
            if let Some(p) = pos {
                b = b.with_position(p);
            }
            if cfg.compact_on_top {
                b = b.with_always_on_top();
            }
            b
        }
    };
    if let Some(icon) = load_icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions { viewport, renderer: eframe::Renderer::Glow, ..Default::default() };
    let res = eframe::run_native(
        "Voiceolator",
        options,
        Box::new(move |cc| {
            let (tx, rx) = std::sync::mpsc::channel();
            hotkeys::start(cc.egui_ctx.clone(), tx.clone());
            ipc::listen(cc.egui_ctx.clone(), tx.clone());
            #[cfg(windows)]
            {
                use raw_window_handle::{HasWindowHandle, RawWindowHandle};
                if let Ok(h) = cc.window_handle() {
                    if let RawWindowHandle::Win32(w) = h.as_raw() {
                        tray::HWND.store(w.hwnd.get(), std::sync::atomic::Ordering::Relaxed);
                    }
                }
            }
            if !files.is_empty() {
                let _ = tx.send(app::UiCommand::AddFiles(files));
            }
            let tray = tray::Tray::start(cc.egui_ctx.clone(), tx);
            Ok(Box::new(app::VoiceolatorApp::new(cc, cfg, rx, tray)))
        }),
    );
    if let Err(e) = res {
        log::error!("window error: {e}");
        show_fatal(&format!("Voiceolator could not open its window:\n{e}\n\nYour graphics driver may not support OpenGL 2.0+. Try updating it."));
    }
}

fn show_fatal(msg: &str) {
    #[cfg(windows)]
    unsafe {
        use windows::core::HSTRING;
        use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
        MessageBoxW(None, &HSTRING::from(msg), &HSTRING::from("Voiceolator"), MB_OK | MB_ICONERROR);
    }
    #[cfg(not(windows))]
    eprintln!("{msg}");
}
