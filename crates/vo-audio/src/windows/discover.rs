//! Discovery of audio devices and of applications that are currently playing audio.

use anyhow::{Context, Result};
use std::collections::HashMap;
use wasapi::{initialize_mta, DeviceEnumerator, DeviceState, Direction, SessionState};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};

#[derive(Clone, Debug)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

#[derive(Clone, Debug)]
pub struct AudioApp {
    /// Root process of the application tree (what process loopback needs).
    pub pid: u32,
    pub exe: String,
    /// Number of audio sessions (e.g. browser audio service + tabs) in this tree.
    pub sessions: usize,
    /// Current peak level 0..1 across its sessions.
    pub peak: f32,
    /// At least one session is active (stream open).
    pub active: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct Proc {
    pub(crate) parent: u32,
    pub(crate) exe: String,
}

pub(crate) fn process_table() -> Result<HashMap<u32, Proc>> {
    let mut map = HashMap::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).context("process snapshot")?;
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        if Process32FirstW(snap, &mut e).is_ok() {
            loop {
                let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
                let exe = String::from_utf16_lossy(&e.szExeFile[..len]);
                map.insert(e.th32ProcessID, Proc { parent: e.th32ParentProcessID, exe });
                if Process32NextW(snap, &mut e).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
    }
    Ok(map)
}

/// Walk up the parent chain while the parent has the same executable name.
/// Browsers render audio in a child "audio service" process; process loopback with
/// INCLUDE_TARGET_PROCESS_TREE must target the main (root) browser process.
pub(crate) fn root_of(pid: u32, table: &HashMap<u32, Proc>) -> u32 {
    let mut cur = pid;
    for _ in 0..16 {
        let Some(p) = table.get(&cur) else { break };
        match table.get(&p.parent) {
            Some(pp) if pp.exe.eq_ignore_ascii_case(&p.exe) && p.parent != cur => cur = p.parent,
            _ => break,
        }
    }
    cur
}

pub fn process_alive(pid: u32) -> bool {
    process_table().map(|t| t.contains_key(&pid)).unwrap_or(false)
}

/// Find the root process of a running application by executable name (e.g. "chrome.exe"
/// or "chrome"). Prefers a tree that currently has an audio session.
pub fn find_app(name: &str) -> Result<Option<AudioApp>> {
    let want = normalise_exe(name);
    let apps = list_audio_apps()?;
    if let Some(a) = apps.iter().filter(|a| normalise_exe(&a.exe) == want).max_by(|a, b| a.peak.total_cmp(&b.peak)) {
        return Ok(Some(a.clone()));
    }
    // Not playing audio yet: still allow capture of the running app (it may start later).
    let table = process_table()?;
    let roots: Vec<u32> = table.iter().filter(|(_, p)| normalise_exe(&p.exe) == want).map(|(&pid, _)| root_of(pid, &table)).collect();
    Ok(roots.first().map(|&pid| AudioApp { pid, exe: table[&pid].exe.clone(), sessions: 0, peak: 0.0, active: false }))
}

pub(crate) fn normalise_exe(s: &str) -> String {
    let s = s.trim().to_ascii_lowercase();
    s.strip_suffix(".exe").map(str::to_string).unwrap_or(s)
}

/// Applications that currently own audio sessions on any active output device,
/// grouped by root process. Excludes this process and the system-sounds session.
pub fn list_audio_apps() -> Result<Vec<AudioApp>> {
    let _ = initialize_mta();
    let table = process_table()?;
    let me = std::process::id();
    let enumerator = DeviceEnumerator::new()?;
    let mut by_root: HashMap<u32, AudioApp> = HashMap::new();
    for dev in &enumerator.get_device_collection(&Direction::Render)? {
        let Ok(dev) = dev else { continue };
        if !matches!(dev.get_state(), Ok(DeviceState::Active)) {
            continue;
        }
        let Ok(mgr) = dev.get_iaudiosessionmanager() else { continue };
        let Ok(sessions) = mgr.get_audiosessionenumerator() else { continue };
        for i in 0..sessions.get_count().unwrap_or(0) {
            let Ok(ctl) = sessions.get_session(i) else { continue };
            let Ok(pid) = ctl.get_process_id() else { continue };
            if pid == 0 {
                continue; // system sounds
            }
            let root = root_of(pid, &table);
            if root == me || pid == me || root_of(me, &table) == root {
                continue; // never offer to capture ourselves (feedback loop)
            }
            let Some(p) = table.get(&root) else { continue };
            let peak = ctl.get_audiometerinformation().and_then(|m| m.get_peak_value()).unwrap_or(0.0);
            let active = matches!(ctl.get_state(), Ok(SessionState::Active));
            let e = by_root.entry(root).or_insert_with(|| AudioApp { pid: root, exe: p.exe.clone(), sessions: 0, peak: 0.0, active: false });
            e.sessions += 1;
            e.peak = e.peak.max(peak);
            e.active |= active;
        }
    }
    let mut v: Vec<AudioApp> = by_root.into_values().collect();
    v.sort_by(|a, b| b.active.cmp(&a.active).then(b.peak.total_cmp(&a.peak)).then(a.exe.cmp(&b.exe)));
    Ok(v)
}

pub fn list_devices(direction: Direction) -> Result<Vec<DeviceInfo>> {
    let _ = initialize_mta();
    let enumerator = DeviceEnumerator::new()?;
    let default_id = enumerator.get_default_device(&direction).and_then(|d| d.get_id()).ok();
    let mut out = Vec::new();
    for dev in &enumerator.get_device_collection(&direction)? {
        let Ok(dev) = dev else { continue };
        if !matches!(dev.get_state(), Ok(DeviceState::Active)) {
            continue;
        }
        let id = dev.get_id()?;
        let name = dev.get_friendlyname().unwrap_or_else(|_| id.clone());
        out.push(DeviceInfo { is_default: Some(&id) == default_id.as_ref(), id, name });
    }
    Ok(out)
}

pub fn list_output_devices() -> Result<Vec<DeviceInfo>> {
    list_devices(Direction::Render)
}

pub fn list_input_devices() -> Result<Vec<DeviceInfo>> {
    list_devices(Direction::Capture)
}

/// Resolve a user-supplied device string (exact id, exact name or case-insensitive
/// substring of the name).
pub fn find_device(direction: Direction, query: &str) -> Result<Option<DeviceInfo>> {
    let devs = list_devices(direction)?;
    let q = query.to_lowercase();
    Ok(devs
        .iter()
        .find(|d| d.id == query || d.name == query)
        .or_else(|| devs.iter().find(|d| d.name.to_lowercase().contains(&q)))
        .cloned())
}

/// Executable name of a running process.
pub fn process_name(pid: u32) -> Option<String> {
    process_table().ok()?.get(&pid).map(|p| p.exe.clone())
}
