//! System tray icon with a small menu, so Voiceolator can run hidden while you play.
//!
//! Windows: notification-area icon (tray-icon crate). Linux: StatusNotifierItem over
//! D-Bus (ksni), shown by KDE, Xfce, Cinnamon, MATE, LXQt, Budgie and by GNOME with the
//! AppIndicator extension (enabled by default on Ubuntu). When no tray is available the
//! app never hides its window, so it cannot get lost.

use crate::app::UiCommand;
use crate::config::ViewMode;
use eframe::egui;
use std::sync::mpsc::Sender;

/// What the tray menu shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrayState {
    pub running: bool,
    pub isolation: bool,
    pub muted: bool,
    pub view: ViewMode,
    pub window_visible: bool,
}

/// RGBA icon, scaled down (box filter) to `size` pixels.
fn icon_rgba(size: u32) -> Option<(Vec<u8>, u32, u32)> {
    let bytes = include_bytes!("../assets/icon.png");
    let mut reader = png::Decoder::new(std::io::Cursor::new(&bytes[..])).read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    if info.color_type != png::ColorType::Rgba || info.width < size {
        return None;
    }
    let (w, h) = (info.width as usize, info.height as usize);
    let f = w / size as usize;
    let mut out = vec![0u8; (size * size * 4) as usize];
    for y in 0..size as usize {
        for x in 0..size as usize {
            let mut acc = [0u32; 4];
            for dy in 0..f {
                for dx in 0..f {
                    let i = ((y * f + dy).min(h - 1) * w + (x * f + dx).min(w - 1)) * 4;
                    for c in 0..4 {
                        acc[c] += buf[i + c] as u32;
                    }
                }
            }
            let o = (y * size as usize + x) * 4;
            for c in 0..4 {
                out[o + c] = (acc[c] / (f * f) as u32) as u8;
            }
        }
    }
    Some((out, size, size))
}

fn send(tx: &Sender<UiCommand>, ctx: &egui::Context, c: UiCommand) {
    let _ = tx.send(c);
    ctx.request_repaint();
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::sync::atomic::{AtomicIsize, Ordering};
    use tray_icon::menu::{CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
    use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

    /// The main window, so a tray click can show it even while egui is idle.
    pub static HWND: AtomicIsize = AtomicIsize::new(0);

    pub fn show_native() {
        use windows::Win32::Foundation::HWND as H;
        use windows::Win32::UI::WindowsAndMessaging::{SetForegroundWindow, ShowWindow, SW_RESTORE};
        let h = HWND.load(Ordering::Relaxed);
        if h != 0 {
            unsafe {
                let w = H(h as *mut core::ffi::c_void);
                let _ = ShowWindow(w, SW_RESTORE);
                let _ = SetForegroundWindow(w);
            }
        }
    }

    pub struct Imp {
        _tray: TrayIcon,
        show: MenuItem,
        start: MenuItem,
        iso: CheckMenuItem,
        mute: CheckMenuItem,
        views: [CheckMenuItem; 3],
    }

    pub fn start(ctx: egui::Context, tx: Sender<UiCommand>) -> anyhow::Result<Imp> {
        let show = MenuItem::with_id("show", "Hide window", true, None);
        let start = MenuItem::with_id("startstop", "Start listening", true, None);
        let iso = CheckMenuItem::with_id("iso", "Voice isolation", true, true, None);
        let mute = CheckMenuItem::with_id("mute", "Mute", true, false, None);
        let views = [
            CheckMenuItem::with_id("view_full", ViewMode::Full.label(), true, true, None),
            CheckMenuItem::with_id("view_slim", ViewMode::Slim.label(), true, false, None),
            CheckMenuItem::with_id("view_pill", ViewMode::Pill.label(), true, false, None),
        ];
        let view_items: Vec<&dyn IsMenuItem> = views.iter().map(|v| v as &dyn IsMenuItem).collect();
        let view = Submenu::with_items("Window", true, &view_items)?;
        let quit = MenuItem::with_id("quit", "Quit Voiceolator", true, None);
        let (s1, s2, s3) = (PredefinedMenuItem::separator(), PredefinedMenuItem::separator(), PredefinedMenuItem::separator());
        let menu = Menu::with_items(&[&show, &s1, &start, &iso, &mute, &s2, &view, &s3, &quit])?;
        let (rgba, w, h) = icon_rgba(32).ok_or_else(|| anyhow::anyhow!("icon"))?;
        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("Voiceolator")
            .with_icon(tray_icon::Icon::from_rgba(rgba, w, h)?)
            .with_menu_on_left_click(false)
            .build()?;
        {
            let (tx, ctx) = (tx.clone(), ctx.clone());
            TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
                if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                    send(&tx, &ctx, UiCommand::ToggleWindow);
                }
            }));
        }
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let c = match e.id.as_ref() {
                "show" => UiCommand::ToggleWindow,
                "startstop" => UiCommand::StartStop,
                "iso" => UiCommand::ToggleIsolation,
                "mute" => UiCommand::ToggleMute,
                "view_full" => UiCommand::SetView(ViewMode::Full),
                "view_slim" => UiCommand::SetView(ViewMode::Slim),
                "view_pill" => UiCommand::SetView(ViewMode::Pill),
                "quit" => UiCommand::Quit,
                _ => return,
            };
            send(&tx, &ctx, c);
        }));
        Ok(Imp { _tray: tray, show, start, iso, mute, views })
    }

    impl Imp {
        pub fn available(&self) -> bool {
            true
        }
        pub fn update(&self, s: &TrayState) {
            self.show.set_text(if s.window_visible { "Hide window" } else { "Show Voiceolator" });
            self.start.set_text(if s.running { "Stop listening" } else { "Start listening" });
            self.iso.set_checked(s.isolation);
            self.mute.set_checked(s.muted);
            for (i, m) in [ViewMode::Full, ViewMode::Slim, ViewMode::Pill].into_iter().enumerate() {
                self.views[i].set_checked(s.view == m);
            }
            let tip = match (s.running, s.isolation, s.muted) {
                (false, ..) => "Voiceolator: stopped",
                (true, _, true) => "Voiceolator: muted",
                (true, true, _) => "Voiceolator: voice only",
                (true, false, _) => "Voiceolator: original sound",
            };
            let _ = self._tray.set_tooltip(Some(tip));
        }
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use ksni::blocking::{Handle, TrayMethods};
    use ksni::menu::{CheckmarkItem, RadioGroup, RadioItem, StandardItem, SubMenu};
    use ksni::MenuItem;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    pub fn show_native() {}

    struct VoTray {
        tx: Sender<UiCommand>,
        ctx: egui::Context,
        state: TrayState,
        icon: Vec<ksni::Icon>,
        online: Arc<AtomicBool>,
    }

    impl VoTray {
        fn cmd(&self, c: UiCommand) {
            send(&self.tx, &self.ctx, c);
        }
    }

    impl ksni::Tray for VoTray {
        fn id(&self) -> String {
            "voiceolator".into()
        }
        fn title(&self) -> String {
            "Voiceolator".into()
        }
        fn category(&self) -> ksni::Category {
            ksni::Category::ApplicationStatus
        }
        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            self.icon.clone()
        }
        fn tool_tip(&self) -> ksni::ToolTip {
            let s = &self.state;
            let d = match (s.running, s.isolation, s.muted) {
                (false, ..) => "Stopped",
                (true, _, true) => "Muted",
                (true, true, _) => "Voice only",
                (true, false, _) => "Original sound",
            };
            ksni::ToolTip { title: "Voiceolator".into(), description: d.into(), icon_name: String::new(), icon_pixmap: Vec::new() }
        }
        fn activate(&mut self, _x: i32, _y: i32) {
            self.cmd(UiCommand::ToggleWindow);
        }
        fn menu(&self) -> Vec<MenuItem<Self>> {
            let s = self.state;
            let views = [ViewMode::Full, ViewMode::Slim, ViewMode::Pill];
            vec![
                StandardItem { label: if s.window_visible { "Hide window".into() } else { "Show Voiceolator".into() }, activate: Box::new(|t: &mut Self| t.cmd(UiCommand::ToggleWindow)), ..Default::default() }.into(),
                MenuItem::Separator,
                StandardItem { label: if s.running { "Stop listening".into() } else { "Start listening".into() }, activate: Box::new(|t: &mut Self| t.cmd(UiCommand::StartStop)), ..Default::default() }.into(),
                CheckmarkItem { label: "Voice isolation".into(), checked: s.isolation, activate: Box::new(|t: &mut Self| t.cmd(UiCommand::ToggleIsolation)), ..Default::default() }.into(),
                CheckmarkItem { label: "Mute".into(), checked: s.muted, activate: Box::new(|t: &mut Self| t.cmd(UiCommand::ToggleMute)), ..Default::default() }.into(),
                MenuItem::Separator,
                SubMenu {
                    label: "Window".into(),
                    submenu: vec![RadioGroup {
                        selected: views.iter().position(|v| *v == s.view).unwrap_or(0),
                        select: Box::new(move |t: &mut Self, i| t.cmd(UiCommand::SetView(views[i.min(2)]))),
                        options: views.iter().map(|v| RadioItem { label: v.label().into(), ..Default::default() }).collect(),
                    }
                    .into()],
                    ..Default::default()
                }
                .into(),
                MenuItem::Separator,
                StandardItem { label: "Quit Voiceolator".into(), activate: Box::new(|t: &mut Self| t.cmd(UiCommand::Quit)), ..Default::default() }.into(),
            ]
        }
        fn watcher_online(&self) {
            self.online.store(true, Ordering::Relaxed);
        }
        fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
            // The panel went away (e.g. restarted): make sure the window is reachable.
            self.online.store(false, Ordering::Relaxed);
            let _ = self.tx.send(UiCommand::ShowWindow);
            self.ctx.request_repaint();
            true
        }
    }

    pub struct Imp {
        handle: Handle<VoTray>,
        online: Arc<AtomicBool>,
    }

    pub fn start(ctx: egui::Context, tx: Sender<UiCommand>) -> anyhow::Result<Imp> {
        // ARGB32, network byte order
        let icon = [22u32, 32, 48, 64]
            .into_iter()
            .filter_map(icon_rgba)
            .map(|(rgba, w, h)| {
                let data = rgba.chunks_exact(4).flat_map(|p| [p[3], p[0], p[1], p[2]]).collect();
                ksni::Icon { width: w as i32, height: h as i32, data }
            })
            .collect();
        let online = Arc::new(AtomicBool::new(true));
        let state = TrayState { running: false, isolation: true, muted: false, view: ViewMode::Full, window_visible: true };
        let handle = VoTray { tx, ctx, state, icon, online: online.clone() }.spawn().map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(Imp { handle, online })
    }

    impl Imp {
        pub fn available(&self) -> bool {
            self.online.load(Ordering::Relaxed) && !self.handle.is_closed()
        }
        pub fn update(&self, s: &TrayState) {
            let s = *s;
            self.handle.update(move |t| t.state = s);
        }
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod imp {
    use super::*;
    pub fn show_native() {}
    pub struct Imp;
    pub fn start(_ctx: egui::Context, _tx: Sender<UiCommand>) -> anyhow::Result<Imp> {
        anyhow::bail!("no tray on this platform")
    }
    impl Imp {
        pub fn available(&self) -> bool {
            false
        }
        pub fn update(&self, _s: &TrayState) {}
    }
}

#[cfg(windows)]
pub use imp::HWND;

/// Show the window through the OS directly (Windows), for when egui is idle.
pub fn show_native() {
    imp::show_native()
}

pub struct Tray {
    imp: imp::Imp,
    last: Option<TrayState>,
}

impl Tray {
    /// None if this desktop has no system tray.
    pub fn start(ctx: egui::Context, tx: Sender<UiCommand>) -> Option<Tray> {
        if std::env::var_os("VOICEOLATOR_NO_TRAY").is_some() {
            return None;
        }
        match imp::start(ctx, tx) {
            Ok(imp) => Some(Tray { imp, last: None }),
            Err(e) => {
                log::info!("no system tray: {e:#}");
                None
            }
        }
    }

    pub fn available(&self) -> bool {
        self.imp.available()
    }

    pub fn update(&mut self, s: TrayState) {
        if self.last != Some(s) {
            self.last = Some(s);
            self.imp.update(&s);
        }
    }
}
