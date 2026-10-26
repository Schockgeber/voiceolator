//! Minimal synchronous helpers around the PulseAudio client API. Works with PulseAudio and
//! with PipeWire (through its PulseAudio-compatible server, `pipewire-pulse`), which
//! together cover practically every Linux desktop.
//!
//! Every helper opens a short-lived connection (a local socket, ~ms), so the state needs
//! no thread affinity.

use anyhow::{anyhow, bail, Result};
use libpulse_binding as pulse;
use pulse::callbacks::ListResult;
use pulse::context::{Context, FlagSet, State};
use pulse::mainloop::standard::{IterateResult, Mainloop};
use pulse::operation::{Operation, State as OpState};
use std::cell::RefCell;
use std::rc::Rc;

pub struct Conn {
    ml: Mainloop,
    ctx: Context,
}

impl Conn {
    pub fn open() -> Result<Self> {
        let mut ml = Mainloop::new().ok_or_else(|| anyhow!("cannot create PulseAudio mainloop"))?;
        let mut ctx = Context::new(&ml, "Voiceolator").ok_or_else(|| anyhow!("cannot create PulseAudio context"))?;
        ctx.connect(None, FlagSet::NOAUTOSPAWN, None)
            .map_err(|e| anyhow!("no PulseAudio/PipeWire sound server reachable ({e}). Is pipewire-pulse or pulseaudio running?"))?;
        loop {
            match ml.iterate(true) {
                IterateResult::Quit(_) | IterateResult::Err(_) => bail!("sound server connection failed"),
                IterateResult::Success(_) => {}
            }
            match ctx.get_state() {
                State::Ready => break,
                State::Failed | State::Terminated => bail!("sound server refused the connection"),
                _ => {}
            }
        }
        Ok(Self { ml, ctx })
    }

    fn wait<T: ?Sized>(&mut self, op: Operation<T>) -> Result<()> {
        while op.get_state() == OpState::Running {
            if let IterateResult::Err(e) = self.ml.iterate(true) {
                bail!("sound server error: {e}");
            }
        }
        Ok(())
    }

    pub fn server_default_sink(&mut self) -> Result<Option<String>> {
        let out = Rc::new(RefCell::new(None));
        let o = out.clone();
        let op = self.ctx.introspect().get_server_info(move |i| {
            *o.borrow_mut() = i.default_sink_name.as_ref().map(|s| s.to_string());
        });
        self.wait(op)?;
        let v = out.borrow().clone();
        Ok(v)
    }

    pub fn sinks(&mut self) -> Result<Vec<SinkRec>> {
        let out = Rc::new(RefCell::new(Vec::new()));
        let o = out.clone();
        let op = self.ctx.introspect().get_sink_info_list(move |r| {
            if let ListResult::Item(s) = r {
                o.borrow_mut().push(SinkRec {
                    index: s.index,
                    name: s.name.as_ref().map(|x| x.to_string()).unwrap_or_default(),
                    description: s.description.as_ref().map(|x| x.to_string()).unwrap_or_default(),
                    owner_module: s.owner_module,
                    monitor: s.monitor_source_name.as_ref().map(|x| x.to_string()).unwrap_or_default(),
                });
            }
        });
        self.wait(op)?;
        let v = out.borrow().clone();
        Ok(v)
    }

    pub fn sink_inputs(&mut self) -> Result<Vec<InputRec>> {
        let out = Rc::new(RefCell::new(Vec::new()));
        let o = out.clone();
        let op = self.ctx.introspect().get_sink_input_info_list(move |r| {
            if let ListResult::Item(s) = r {
                let p = &s.proplist;
                o.borrow_mut().push(InputRec {
                    index: s.index,
                    sink: s.sink,
                    corked: s.corked,
                    owner_module: s.owner_module,
                    client: s.client,
                    binary: p.get_str("application.process.binary").unwrap_or_default(),
                    app_name: p.get_str("application.name").unwrap_or_default(),
                    pid: p.get_str("application.process.id").and_then(|v| v.parse().ok()).unwrap_or(0),
                    media: s.name.as_ref().map(|x| x.to_string()).unwrap_or_default(),
                });
            }
        });
        self.wait(op)?;
        let v = out.borrow().clone();
        Ok(v)
    }

    pub fn move_input(&mut self, input: u32, sink_name: &str) -> Result<bool> {
        let ok = Rc::new(RefCell::new(false));
        let o = ok.clone();
        let op = self.ctx.introspect().move_sink_input_by_name(input, sink_name, Some(Box::new(move |s| *o.borrow_mut() = s)));
        self.wait(op)?;
        let v = *ok.borrow();
        Ok(v)
    }

    pub fn load_module(&mut self, name: &str, args: &str) -> Result<u32> {
        let idx = Rc::new(RefCell::new(u32::MAX));
        let o = idx.clone();
        let op = self.ctx.introspect().load_module(name, args, move |i| *o.borrow_mut() = i);
        self.wait(op)?;
        let v = *idx.borrow();
        if v == u32::MAX {
            bail!("the sound server refused to load {name}");
        }
        Ok(v)
    }

    pub fn unload_module(&mut self, index: u32) -> Result<bool> {
        let ok = Rc::new(RefCell::new(false));
        let o = ok.clone();
        let op = self.ctx.introspect().unload_module(index, move |s| *o.borrow_mut() = s);
        self.wait(op)?;
        let v = *ok.borrow();
        Ok(v)
    }
}

impl Drop for Conn {
    fn drop(&mut self) {
        self.ctx.disconnect();
    }
}

#[derive(Clone, Debug)]
pub struct SinkRec {
    pub index: u32,
    pub name: String,
    pub description: String,
    pub owner_module: Option<u32>,
    pub monitor: String,
}

#[derive(Clone, Debug)]
pub struct InputRec {
    pub index: u32,
    pub sink: u32,
    pub corked: bool,
    pub owner_module: Option<u32>,
    /// Owning client; `None` for streams created by server modules (e.g. loopbacks).
    pub client: Option<u32>,
    pub binary: String,
    pub app_name: String,
    pub pid: u32,
    pub media: String,
}

impl InputRec {
    /// Name used to identify the application (executable when known).
    pub fn app_key(&self) -> String {
        let b = if !self.binary.is_empty() { &self.binary } else { &self.app_name };
        b.trim().to_lowercase()
    }
}
