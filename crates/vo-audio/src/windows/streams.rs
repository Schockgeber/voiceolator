//! WASAPI capture and render streams (shared mode, 48 kHz float, automatic conversion).

use crate::backend::{CaptureSource, InputFactory, InputStream, OutputFactory, OutputStream, OutputTarget};
use crate::windows::discover;
use anyhow::{bail, Context, Result};
use wasapi::{
    initialize_mta, AudioCaptureClient, AudioClient, AudioRenderClient, DeviceEnumerator, Direction, Handle, SampleType,
    StreamMode, WaveFormat,
};

const SR: usize = 48000;
const CH: usize = 2;

fn format() -> WaveFormat {
    WaveFormat::new(32, 32, &SampleType::Float, SR, CH, None)
}

fn as_bytes(x: &[f32]) -> &[u8] {
    // SAFETY: f32 has no invalid bit patterns and u8 has alignment 1.
    unsafe { std::slice::from_raw_parts(x.as_ptr() as *const u8, std::mem::size_of_val(x)) }
}

pub struct WasapiInput {
    client: AudioClient,
    capture: AudioCaptureClient,
    event: Option<Handle>,
    buf: Vec<u8>,
    latency: usize,
    pid: Option<u32>,
    idle_ms: u32,
}

impl WasapiInput {
    pub fn open(src: &CaptureSource) -> Result<Self> {
        let _ = initialize_mta();
        let fmt = format();
        let (mut client, events, pid) = match src {
            CaptureSource::Process { pid, name } => {
                // The browser may have been restarted (new PID): re-resolve by name.
                let pid = if *pid != 0 && discover::process_alive(*pid) {
                    *pid
                } else {
                    discover::find_app(name)?.with_context(|| format!("{name} is not running"))?.pid
                };
                if pid == std::process::id() {
                    bail!("refusing to capture Voiceolator itself (feedback loop)");
                }
                let c = AudioClient::new_application_loopback_client(pid, true)
                    .context("process loopback capture is not available (requires Windows 10 2004 / build 19041 or newer)")?;
                (c, true, Some(pid))
            }
            CaptureSource::InputDevice { id, .. } => {
                let dev = DeviceEnumerator::new()?.get_device(id).context("input device not found")?;
                (dev.get_iaudioclient()?, true, None)
            }
            CaptureSource::OutputLoopback { id, .. } => {
                let dev = DeviceEnumerator::new()?.get_device(id).context("output device not found")?;
                // Event-driven loopback is unreliable when nothing plays; use polling.
                (dev.get_iaudioclient()?, false, None)
            }
        };
        let mode = if events {
            StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 0 }
        } else {
            StreamMode::PollingShared { autoconvert: true, buffer_duration_hns: 200_000 }
        };
        client.initialize_client(&fmt, &Direction::Capture, &mode).context("initialising capture stream")?;
        let event = if events { Some(client.set_get_eventhandle()?) } else { None };
        let capture = client.get_audiocaptureclient()?;
        // Process-loopback clients don't report a period; assume the engine's 10 ms.
        let latency = client.get_device_period().map(|(def, _)| (def as usize * SR) / 10_000_000).unwrap_or(SR / 100);
        client.start_stream()?;
        Ok(Self { client, capture, event, buf: vec![0u8; SR * CH * 4], latency, pid, idle_ms: 0 })
    }
}

impl Drop for WasapiInput {
    fn drop(&mut self) {
        let _ = self.client.stop_stream();
    }
}

impl InputStream for WasapiInput {
    fn sample_rate(&self) -> u32 {
        SR as u32
    }
    fn channels(&self) -> usize {
        CH
    }
    fn latency_frames(&self) -> usize {
        self.latency
    }
    fn read(&mut self, out: &mut Vec<f32>, timeout_ms: u32) -> Result<usize> {
        match &self.event {
            Some(h) => {
                if h.wait_for_event(timeout_ms).is_err() {
                    // No packets: the app is silent, or it went away.
                    self.idle_ms += timeout_ms;
                    if self.idle_ms >= 2000 {
                        self.idle_ms = 0;
                        if let Some(pid) = self.pid {
                            if !discover::process_alive(pid) {
                                bail!("the captured application exited");
                            }
                        }
                    }
                    return Ok(0);
                }
            }
            None => std::thread::sleep(std::time::Duration::from_millis(5)),
        }
        self.idle_ms = 0;
        let mut total = 0;
        loop {
            let n = self.capture.get_next_packet_size()?.unwrap_or(0) as usize;
            if n == 0 {
                break;
            }
            if self.buf.len() < n * CH * 4 {
                self.buf.resize(n * CH * 4, 0);
            }
            let (frames, info) = self.capture.read_from_device(&mut self.buf)?;
            let frames = frames as usize;
            if info.flags.silent {
                out.extend(std::iter::repeat(0.0).take(frames * CH));
            } else {
                out.extend(self.buf[..frames * CH * 4].chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])));
            }
            total += frames;
        }
        Ok(total)
    }
}

pub struct WasapiOutput {
    client: AudioClient,
    render: AudioRenderClient,
    event: Handle,
    buffer_frames: usize,
    started: bool,
    clock: Option<wasapi::AudioClock>,
    clock_freq: u64,
}

impl WasapiOutput {
    pub fn open(target: &OutputTarget, buffer_ms: f32) -> Result<Self> {
        let _ = initialize_mta();
        let enumerator = DeviceEnumerator::new()?;
        let dev = match &target.id {
            Some(id) => enumerator.get_device(id).with_context(|| format!("output device '{}' not found", target.name))?,
            None => enumerator.get_default_device(&Direction::Render).context("no default output device")?,
        };
        let mut client = dev.get_iaudioclient()?;
        let hns = (buffer_ms.max(10.0) as i64) * 10_000;
        client
            .initialize_client(&format(), &Direction::Render, &StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: hns })
            .context("initialising output stream")?;
        let event = client.set_get_eventhandle()?;
        let render = client.get_audiorenderclient()?;
        let buffer_frames = client.get_buffer_size()? as usize;
        let clock = client.get_audioclock().ok();
        let clock_freq = clock.as_ref().and_then(|c| c.get_frequency().ok()).unwrap_or(0);
        Ok(Self { client, render, event, buffer_frames, started: false, clock, clock_freq })
    }
}

impl Drop for WasapiOutput {
    fn drop(&mut self) {
        let _ = self.client.stop_stream();
    }
}

impl OutputStream for WasapiOutput {
    fn sample_rate(&self) -> u32 {
        SR as u32
    }
    fn channels(&self) -> usize {
        CH
    }
    fn wait(&mut self, timeout_ms: u32) -> Result<()> {
        if !self.started {
            return Ok(()); // first call: pre-fill before starting
        }
        if self.event.wait_for_event(timeout_ms).is_err() {
            bail!("output device stopped responding");
        }
        Ok(())
    }
    fn writable_frames(&mut self) -> Result<usize> {
        Ok(self.client.get_available_space_in_frames()? as usize)
    }
    fn queued_frames(&mut self) -> Result<usize> {
        Ok(self.client.get_current_padding()? as usize)
    }
    fn write(&mut self, data: &[f32]) -> Result<()> {
        let frames = data.len() / CH;
        self.render.write_to_device(frames, as_bytes(data), None)?;
        if !self.started {
            self.client.start_stream()?;
            self.started = true;
        }
        Ok(())
    }
    fn play_time_100ns(&mut self, frame_index: u64) -> Option<i64> {
        let clock = self.clock.as_ref()?;
        if self.clock_freq == 0 || !self.started {
            return None;
        }
        let (pos, qpc) = clock.get_position().ok()?;
        let played_s = pos as f64 / self.clock_freq as f64;
        let frame_s = frame_index as f64 / SR as f64;
        Some(qpc as i64 + ((frame_s - played_s) * 1e7) as i64)
    }
}

impl WasapiOutput {
    pub fn buffer_frames(&self) -> usize {
        self.buffer_frames
    }
}

pub fn input_factory(src: CaptureSource) -> InputFactory {
    Box::new(move || Ok(Box::new(WasapiInput::open(&src)?) as Box<dyn InputStream>))
}

pub fn output_factory(target: OutputTarget, buffer_ms: f32) -> OutputFactory {
    Box::new(move || Ok(Box::new(WasapiOutput::open(&target, buffer_ms)?) as Box<dyn OutputStream>))
}
