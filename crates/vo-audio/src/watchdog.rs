//! Crash safety for the system changes Voiceolator makes while running (a turned-down
//! app on Windows, a private capture sink on Linux).
//!
//! The first time such a change is made, a tiny helper copy of the running executable is
//! started with `--vo-watchdog <pid>`. It does nothing but wait for Voiceolator to exit;
//! if Voiceolator ends without cleaning up (crash, killed from the task manager), the
//! helper undoes the changes immediately, so the browser's sound comes back without
//! having to restart Voiceolator. Executables must call [`handle_args`] first in `main`.

use std::sync::OnceLock;
use std::time::Duration;

static SPAWNED: OnceLock<()> = OnceLock::new();

/// Start the watchdog once per process (no-op afterwards or if it can't be started).
pub fn spawn_once() {
    SPAWNED.get_or_init(|| {
        let Ok(exe) = std::env::current_exe() else { return };
        let mut cmd = std::process::Command::new(exe);
        cmd.arg("--vo-watchdog")
            .arg(std::process::id().to_string())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        if let Err(e) = cmd.spawn() {
            log::warn!("could not start the crash watchdog: {e}");
        }
    });
}

fn parent_alive(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        std::path::Path::new(&format!("/proc/{pid}")).exists()
    }
    #[cfg(windows)]
    {
        crate::windows::discover::process_alive(pid)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = pid;
        false
    }
}

fn recover() {
    #[cfg(target_os = "linux")]
    {
        let _ = crate::linux::recover();
    }
    #[cfg(windows)]
    {
        let _ = crate::windows::duck::recover();
    }
}

/// If the process was started as a watchdog, run it and exit. Call first in `main`.
pub fn handle_args() {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("--vo-watchdog") {
        return;
    }
    let Some(pid) = args.next().and_then(|p| p.parse::<u32>().ok()) else { std::process::exit(2) };
    while parent_alive(pid) {
        std::thread::sleep(Duration::from_millis(500));
    }
    // Normal exits clean up themselves; this only finds work after a crash.
    recover();
    std::process::exit(0);
}
