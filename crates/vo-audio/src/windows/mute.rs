//! Audio-session enumeration (IAudioSessionManager2 → IAudioSessionControl2 →
//! ISimpleAudioVolume) shared by the ducking code.
//!
//! Note: muting an app in the mixer also silences its process-loopback capture (Windows
//! applies the session volume before the loopback tap), so Voiceolator *turns the app
//! down and compensates* instead of muting it (see `duck.rs`). [`unmute_session_ids`]
//! only remains to clean up after version 0.9.0, which used mute.

use anyhow::{Context, Result};
use std::collections::HashSet;
use windows::core::Interface;
use windows::Win32::Media::Audio::{
    eRender, IAudioSessionControl2, IAudioSessionManager2, IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator,
    DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};

pub(crate) struct Session {
    pub(crate) id: String,
    pub(crate) pid: u32,
    pub(crate) volume: ISimpleAudioVolume,
}

pub(crate) fn sessions() -> Result<Vec<Session>> {
    let _ = wasapi::initialize_mta();
    let mut out = Vec::new();
    unsafe {
        let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).context("device enumerator")?;
        let devs = en.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
        for d in 0..devs.GetCount()? {
            let Ok(dev) = devs.Item(d) else { continue };
            let Ok(mgr) = dev.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else { continue };
            let Ok(list) = mgr.GetSessionEnumerator() else { continue };
            for i in 0..list.GetCount().unwrap_or(0) {
                let Ok(ctl) = list.GetSession(i) else { continue };
                let Ok(ctl2) = ctl.cast::<IAudioSessionControl2>() else { continue };
                let Ok(pid) = ctl2.GetProcessId() else { continue };
                let Ok(idp) = ctl2.GetSessionIdentifier() else { continue };
                let id = idp.to_string().unwrap_or_default();
                windows::Win32::System::Com::CoTaskMemFree(Some(idp.0 as *const _));
                let Ok(volume) = ctl.cast::<ISimpleAudioVolume>() else { continue };
                out.push(Session { id, pid, volume });
            }
        }
    }
    Ok(out)
}

/// Unmute sessions by stable identifier (used after an unclean exit).
pub fn unmute_session_ids(ids: &[String]) -> Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    let want: HashSet<&str> = ids.iter().map(String::as_str).collect();
    let mut n = 0;
    for s in sessions()? {
        if want.contains(s.id.as_str()) {
            unsafe {
                if s.volume.SetMute(false, std::ptr::null()).is_ok() {
                    n += 1;
                }
            }
        }
    }
    Ok(n)
}

/// Mute/unmute all sessions of one specific process id (used for the latency beacon).
pub fn set_pid_muted(pid: u32, mute: bool) -> Result<usize> {
    let mut n = 0;
    for s in sessions()? {
        if s.pid == pid {
            unsafe {
                if s.volume.SetMute(mute, std::ptr::null()).is_ok() {
                    n += 1;
                }
            }
        }
    }
    Ok(n)
}
