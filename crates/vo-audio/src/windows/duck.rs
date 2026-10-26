//! "Hide the original sound" without changing any Windows settings.
//!
//! The source app's audio sessions are turned down to −70 dB in the mixer (documented
//! ISimpleAudioVolume API) while Voiceolator runs, and the capture is compensated (see
//! [`crate::duckcomp`]). The original volumes are restored when capture stops, and are
//! also written to a small recovery file so that a crash can't leave the browser silent:
//! [`recover`] restores them at the next start.

use super::discover::{normalise_exe, process_table, root_of};
use super::mute::sessions;
use crate::backend::{CaptureSource, InputFactory, InputStream};
use crate::duckcomp::{DuckCompensator, DuckMode, DUCK_LEVEL};
use anyhow::Result;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq)]
pub enum DuckTarget {
    /// All sessions of an app, matched by the executable of their root process.
    Exe(String),
    /// All sessions of one process tree.
    Pid(u32),
}

fn recovery_file() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| ".".into());
    base.join("Voiceolator").join("hidden-sessions.txt")
}

/// Restore session volumes left turned down by a previous run that did not exit cleanly.
pub fn recover() -> Result<usize> {
    let path = recovery_file();
    let Ok(txt) = std::fs::read_to_string(&path) else { return Ok(0) };
    let want: HashMap<String, f32> = txt
        .lines()
        .filter_map(|l| {
            let (v, id) = l.split_once('\t')?;
            Some((id.to_string(), v.parse().ok()?))
        })
        .collect();
    let mut n = 0;
    for s in sessions()? {
        if let Some(&v) = want.get(&s.id) {
            unsafe {
                if s.volume.SetMasterVolume(v, std::ptr::null()).is_ok() {
                    n += 1;
                }
            }
        }
    }
    let _ = std::fs::remove_file(path);
    Ok(n)
}

/// Turns an app's sessions down and remembers their original volume.
pub struct SessionDucker {
    target: DuckTarget,
    original: HashMap<String, f32>,
}

impl SessionDucker {
    pub fn new(target: DuckTarget) -> Self {
        Self { target, original: HashMap::new() }
    }

    fn matches(&self, pid: u32, table: &HashMap<u32, super::discover::Proc>) -> bool {
        match &self.target {
            DuckTarget::Pid(p) => pid == *p || root_of(pid, table) == *p,
            DuckTarget::Exe(exe) => table.get(&root_of(pid, table)).map(|p| normalise_exe(&p.exe) == normalise_exe(exe)).unwrap_or(false),
        }
    }

    /// Turn down every matching session that isn't already. Returns how many changed.
    pub fn apply(&mut self) -> Result<usize> {
        let table = process_table()?;
        let mut changed = 0;
        for s in sessions()? {
            if !self.matches(s.pid, &table) {
                continue;
            }
            unsafe {
                let cur = s.volume.GetMasterVolume().unwrap_or(1.0);
                if (cur - DUCK_LEVEL).abs() < 1e-6 {
                    continue;
                }
                // If the user moves the app's slider while we run, keep their new value
                // as the one to restore.
                self.original.insert(s.id.clone(), cur);
                if s.volume.SetMasterVolume(DUCK_LEVEL, std::ptr::null()).is_ok() {
                    changed += 1;
                }
            }
        }
        if changed > 0 {
            self.save_recovery();
            crate::watchdog::spawn_once();
        }
        Ok(changed)
    }

    fn save_recovery(&self) {
        let p = recovery_file();
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let txt: String = self.original.iter().map(|(id, v)| format!("{v}\t{id}\n")).collect();
        let _ = std::fs::write(p, txt);
    }

    pub fn restore(&mut self) {
        if self.original.is_empty() {
            return;
        }
        if let Ok(list) = sessions() {
            for s in list {
                if let Some(&v) = self.original.get(&s.id) {
                    unsafe {
                        let _ = s.volume.SetMasterVolume(v, std::ptr::null());
                    }
                }
            }
        }
        self.original.clear();
        let _ = std::fs::remove_file(recovery_file());
    }
}

impl Drop for SessionDucker {
    fn drop(&mut self) {
        self.restore();
    }
}

/// Shared between the capture stream(s) and the UI for status display.
pub struct DuckShared {
    ducker: Mutex<SessionDucker>,
    comp: Mutex<DuckCompensator>,
}

impl DuckShared {
    pub fn mode(&self) -> DuckMode {
        self.comp.lock().mode()
    }
    pub fn restore(&self) {
        self.ducker.lock().restore();
    }
    /// Turn down the target's sessions now. Returns how many were changed.
    pub fn apply_now(&self) -> Result<usize> {
        self.ducker.lock().apply()
    }
}

struct DuckingInput {
    inner: Box<dyn InputStream>,
    shared: Arc<DuckShared>,
    last_apply: Instant,
}

impl InputStream for DuckingInput {
    fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }
    fn channels(&self) -> usize {
        self.inner.channels()
    }
    fn latency_frames(&self) -> usize {
        self.inner.latency_frames()
    }
    fn read(&mut self, out: &mut Vec<f32>, timeout_ms: u32) -> Result<usize> {
        let start = out.len();
        let n = self.inner.read(out, timeout_ms)?;
        let mut reapply = self.last_apply.elapsed() > Duration::from_millis(500);
        if n > 0 {
            let ms = n as f32 * 1000.0 / self.inner.sample_rate() as f32;
            reapply |= self.shared.comp.lock().process(&mut out[start..], ms);
        }
        if reapply {
            self.last_apply = Instant::now();
            if let Err(e) = self.shared.ducker.lock().apply() {
                log::warn!("could not turn down the original audio: {e:#}");
            }
        }
        Ok(n)
    }
}

/// Capture `src`, hiding the original sound of `target` as described above.
/// Returns the factory and a handle for status/restore. Volumes are restored when the
/// last handle (factory + returned Arc) is dropped.
pub fn ducked_input_factory(src: CaptureSource, target: DuckTarget) -> (InputFactory, Arc<DuckShared>) {
    let shared = Arc::new(DuckShared { ducker: Mutex::new(SessionDucker::new(target)), comp: Mutex::new(DuckCompensator::default()) });
    let s2 = shared.clone();
    let f: InputFactory = Box::new(move || {
        // Turn the app down before the stream starts, so the mode check sees ducked audio.
        let _ = s2.ducker.lock().apply();
        s2.comp.lock().warm_up();
        let inner = super::streams::WasapiInput::open(&src)?;
        Ok(Box::new(DuckingInput { inner: Box::new(inner), shared: s2.clone(), last_apply: Instant::now() }) as Box<dyn InputStream>)
    });
    (f, shared)
}
