//! The window: winit for events, softbuffer for damage-only CPU presents.
//! No frame loop: the event loop waits, and a redraw is requested only when
//! the UI recorded damage.

use crate::platform;
use crate::ui::SinkFactory;
use crate::ui::{Chrome, Key, Ui, WinAction, load_fonts};
use crate::work::{self, Msg, Sink};
use crate::{Args, trace};
use ffwd_render::{Canvas, TextEngine};
use softbuffer::{Context, Surface};
use std::num::NonZeroU32;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key as WKey, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

pub enum UserEvent {
    /// A loader message for the tab with this id (0: not tab-specific).
    Msg(u64, Msg),
    SmokeTimeout,
    /// The Windows frame changed the maximise button or window state.
    #[cfg_attr(not(windows), allow(dead_code))]
    Frame,
    UpdateFound(Result<crate::update::Outcome, String>),
    UpdateFinished(Result<(), String>),
}

struct Loaded {
    te: TextEngine,
    fonts: crate::ui::Fonts,
    settings: crate::settings::Settings,
    state: crate::settings::Settings,
}

struct Gfx {
    window: Rc<Window>,
    surface: Surface<Rc<Window>, Rc<Window>>,
    hwnd: isize,
}

struct App {
    t0: Instant,
    smoke: bool,
    ui: Option<Ui>,
    font_thread: Option<std::thread::JoinHandle<Option<Loaded>>>,
    sinks: SinkFactory,
    pending: Vec<(u64, Msg)>,
    /// The repository given on the command line, already loading as tab 1.
    first: Option<std::path::PathBuf>,
    title: String,
    gfx: Option<Gfx>,
    size: (usize, usize),
    mods: ModifiersState,
    cursor: (i32, i32),
    exit_code: i32,
    smoke_reported: bool,
    proxy: EventLoopProxy<UserEvent>,
    title_sent: Option<crate::ui::TitleGeometry>,
    update_offer: Option<ffwd_update::Candidate>,
    updates_started: bool,
    update_installing: bool,
    updates_forced: bool,
}

pub fn run(args: &Args, t0: Instant) -> i32 {
    let event_loop = match EventLoop::<UserEvent>::with_user_event().build() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("ffwd: cannot open a window: {e}");
            return 1;
        }
    };
    trace(t0, "event loop built");
    event_loop.set_control_flow(ControlFlow::Wait);
    let proxy = Arc::new(Mutex::new(event_loop.create_proxy()));
    let sinks: SinkFactory = Arc::new(move |id| {
        let proxy = proxy.clone();
        let sink: Sink = Arc::new(move |m| {
            if let Ok(p) = proxy.lock() {
                let _ = p.send_event(UserEvent::Msg(id, m));
            }
        });
        sink
    });
    // Git work for the repository on the command line starts before the
    // window exists: first frame first, data behind it. Saved tabs load
    // when they are activated.
    if let Some(repo) = &args.repo {
        work::start(repo.clone(), &sinks(1));
    }
    if args.smoke {
        let p = Mutex::new(event_loop.create_proxy());
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(30));
            if let Ok(p) = p.lock() {
                let _ = p.send_event(UserEvent::SmokeTimeout);
            }
        });
    }
    let smoke = args.smoke;
    let font_thread = Some(std::thread::spawn(move || {
        let mut te = TextEngine::new();
        // Settings and session state are read here too, so file I/O stays
        // off the UI thread.
        let settings = crate::settings::Settings::load();
        // Agent smoke runs neither read nor write the user's session.
        let state = if smoke { crate::settings::Settings::parse("") } else { crate::settings::Settings::load_state() };
        let f = load_fonts(&mut te).map(|fonts| Loaded { te, fonts, settings, state });
        trace(t0, "fonts parsed (worker)");
        f
    }));
    let mut app = App {
        t0,
        smoke: args.smoke,
        ui: None,
        font_thread,
        sinks,
        pending: Vec::new(),
        first: args.repo.clone(),
        title: String::new(),
        gfx: None,
        size: (0, 0),
        mods: ModifiersState::empty(),
        cursor: (0, 0),
        exit_code: 0,
        smoke_reported: false,
        proxy: event_loop.create_proxy(),
        title_sent: None,
        update_offer: None,
        updates_started: false,
        update_installing: false,
        updates_forced: false,
    };
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("ffwd: event loop failed: {e}");
        return 1;
    }
    app.exit_code
}

impl App {
    fn request(&self) {
        if let (Some(g), Some(ui)) = (&self.gfx, &self.ui)
            && !ui.damage.is_empty()
        {
            g.window.request_redraw();
        }
    }

    fn render(&mut self) {
        let (Some(gfx), Some(ui)) = (&mut self.gfx, &mut self.ui) else { return };
        let (w, h) = self.size;
        if w == 0 || h == 0 || ui.damage.is_empty() && ui.stats.presents > 0 {
            return;
        }
        let Ok(mut buf) = gfx.surface.buffer_mut() else { return };
        let force = buf.age() == 0;
        let mut cv = Canvas::new(&mut buf[..], w, h);
        let mut painted = ui.paint(&mut cv, force);
        // The frame's hit-test follows the painted title row.
        let geo = ui.title_geometry();
        if self.title_sent.as_ref() != Some(&geo) {
            #[cfg(windows)]
            crate::frame::set_caption(crate::frame::Caption {
                height: geo.height,
                controls: geo.controls.clone(),
                buttons_left: geo.buttons_left,
                max_button: geo.max_button,
            });
            self.title_sent = Some(geo);
        }
        if let Some((area, dy)) = painted.copy
            && !platform::screen_copy(gfx.hwnd, area, dy)
        {
            painted.rects.push(area);
        }
        let rects: Vec<softbuffer::Rect> = painted
            .rects
            .iter()
            .filter_map(|r| {
                Some(softbuffer::Rect {
                    x: r.x.max(0) as u32,
                    y: r.y.max(0) as u32,
                    width: NonZeroU32::new(r.w.max(0) as u32)?,
                    height: NonZeroU32::new(r.h.max(0) as u32)?,
                })
            })
            .collect();
        if rects.is_empty() {
            return;
        }
        if let Err(e) = buf.present_with_damage(&rects) {
            eprintln!("ffwd: present failed: {e}");
        }
        if ui.stats.first_frame_ms.is_none() {
            ui.stats.first_frame_ms = Some(self.t0.elapsed().as_secs_f64() * 1000.0);
            trace(self.t0, "first frame presented");
        }
        trace(
            self.t0,
            &format!(
                "present #{}: {} rect(s), {} px, paint {:.2} ms{}",
                ui.stats.presents,
                rects.len(),
                ui.stats.last_damage_px,
                ui.stats.last_paint_ms,
                if painted.copy.is_some() { ", scroll by copy" } else { "" }
            ),
        );
    }

    fn smoke_check(&mut self, el: &ActiveEventLoop) {
        if !self.smoke || self.smoke_reported {
            return;
        }
        let Some(ui) = &self.ui else { return };
        let diff_ready = ui.r.diff.is_some() || ui.r.details.as_ref().is_some_and(|d| d.files.is_empty());
        if ui.r.log_done && ui.r.details.is_some() && diff_ready && ui.damage.is_empty() {
            self.smoke_reported = true;
            println!(
                "{{\"first_frame_ms\":{:.1},\"commits\":{},\"log_ms\":{:.1},\"first_rows_ms\":{:.1},\"presents\":{},\"remote\":{}}}",
                ui.stats.first_frame_ms.unwrap_or(-1.0),
                ui.r.commits.len(),
                ui.r.log_ms,
                ui.r.first_batch_ms.unwrap_or(-1.0),
                ui.stats.presents,
                ui.remote
            );
            el.exit();
        }
    }

    fn key(&mut self, ev: &winit::event::KeyEvent) {
        if ev.state != ElementState::Pressed {
            return;
        }
        let Some(ui) = &mut self.ui else { return };
        let ctrl = self.mods.control_key() || self.mods.super_key();
        let k = match &ev.logical_key {
            WKey::Named(n) => match n {
                NamedKey::ArrowUp => Some(Key::Up),
                NamedKey::ArrowLeft => Some(Key::Left),
                NamedKey::ArrowRight => Some(Key::Right),
                NamedKey::Delete => Some(Key::Delete),
                NamedKey::Backspace if ctrl => Some(Key::CtrlBackspace),
                NamedKey::ArrowDown => Some(Key::Down),
                NamedKey::PageUp => Some(Key::PageUp),
                NamedKey::PageDown => Some(Key::PageDown),
                NamedKey::Home => Some(Key::Home),
                NamedKey::End => Some(Key::End),
                NamedKey::Tab if ctrl && self.mods.shift_key() => Some(Key::PrevTab),
                NamedKey::Tab if ctrl => Some(Key::NextTab),
                NamedKey::Tab if self.mods.shift_key() => Some(Key::BackTab),
                NamedKey::Tab => Some(Key::Tab),
                NamedKey::Enter => Some(Key::Enter),
                NamedKey::Escape => Some(Key::Esc),
                NamedKey::Backspace => Some(Key::Backspace),
                NamedKey::Space if !ctrl => Some(Key::Char(' ')),
                _ => None,
            },
            WKey::Character(s) if ctrl && (s.eq_ignore_ascii_case("k") || s.eq_ignore_ascii_case("p")) => {
                Some(Key::Palette)
            }
            WKey::Character(s) if ctrl => {
                // Shift changes the produced character ("T" for t); the key
                // without modifiers says which key it was.
                use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
                let base = ev.key_without_modifiers();
                let c = match &base {
                    WKey::Character(b) => b.chars().next(),
                    _ => s.chars().next(),
                }
                .map(|c| c.to_ascii_lowercase());
                c.map(|c| if self.mods.shift_key() { Key::CtrlShift(c) } else { Key::Ctrl(c) })
            }
            WKey::Character(s) => {
                for c in s.chars().filter(|c| !c.is_control()) {
                    ui.key(Key::Char(c));
                }
                None
            }
            _ => None,
        };
        if let Some(k) = k {
            ui.key(k);
        }
        self.after_input();
    }

    /// Work the UI asked for that needs the window or the event loop.
    fn after_input(&mut self) {
        self.process_update_request();
        if self.ui.as_mut().is_some_and(|ui| ui.take_update_check_requested()) {
            self.spawn_update_check(true);
        }
        if let (Some(ui), Some(g)) = (&mut self.ui, &self.gfx)
            && std::mem::take(&mut ui.title_dirty)
        {
            let t = ui.window_title();
            if t != self.title {
                g.window.set_title(&t);
                self.title = t;
            }
        }
    }

    /// One update check per run, after the first frame; never in agent runs.
    fn start_update_check(&mut self) {
        if self.updates_started || self.smoke {
            return;
        }
        self.updates_started = true;
        self.spawn_update_check(false);
    }

    fn spawn_update_check(&mut self, force: bool) {
        let Some(mut policy) = self.ui.as_ref().map(|ui| ui.update_policy()) else { return };
        policy.force = force;
        self.updates_forced |= force;
        let proxy = Mutex::new(self.proxy.clone());
        crate::update::spawn_check(policy, move |r| {
            if let Ok(p) = proxy.lock() {
                let _ = p.send_event(UserEvent::UpdateFound(r));
            }
        });
    }

    fn process_update_request(&mut self) {
        if !self.ui.as_mut().is_some_and(|ui| ui.take_update_requested()) || self.update_installing {
            return;
        }
        let Some(c) = self.update_offer.clone() else { return };
        self.update_installing = true;
        if let Some(ui) = &mut self.ui {
            ui.set_update_message(format!("Downloading ffwd {}…", c.version));
        }
        let proxy = Mutex::new(self.proxy.clone());
        crate::update::spawn_install(c, move |r| {
            if let Ok(p) = proxy.lock() {
                let _ = p.send_event(UserEvent::UpdateFinished(r));
            }
        });
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        #[allow(unused_mut)]
        let mut attrs = Window::default_attributes()
            .with_title("ffwd")
            .with_inner_size(LogicalSize::new(1440.0, 900.0))
            .with_min_inner_size(LogicalSize::new(720.0, 420.0))
            .with_visible(false);
        #[cfg(windows)]
        {
            use winit::platform::windows::WindowAttributesExtWindows;
            attrs = attrs.with_drag_and_drop(false);
            if self.smoke {
                attrs = attrs.with_skip_taskbar(true).with_active(false);
            }
        }
        #[cfg(target_os = "macos")]
        {
            // The top bar runs under the traffic lights (Chrome::MacInset).
            use winit::platform::macos::WindowAttributesExtMacOS;
            attrs = attrs.with_titlebar_transparent(true).with_fullsize_content_view(true).with_title_hidden(true);
        }
        attrs = attrs.with_window_icon(crate::icon::window_icon());
        let window = match el.create_window(attrs) {
            Ok(w) => Rc::new(w),
            Err(e) => {
                eprintln!("ffwd: cannot create window: {e}");
                self.exit_code = 1;
                el.exit();
                return;
            }
        };
        trace(self.t0, "window created");
        let surface = Context::new(window.clone()).and_then(|ctx| Surface::new(&ctx, window.clone()));
        let surface = match surface {
            Ok(s) => s,
            Err(e) => {
                eprintln!("ffwd: cannot create a drawing surface: {e}");
                self.exit_code = 1;
                el.exit();
                return;
            }
        };
        let hwnd = match raw_window_handle::HasWindowHandle::window_handle(&*window).map(|h| h.as_raw()) {
            Ok(raw_window_handle::RawWindowHandle::Win32(h)) => h.hwnd.get(),
            _ => 0,
        };
        trace(self.t0, "surface created");
        platform::dark_title_bar(hwnd);
        // Before reading the size: removing the caption grows the client area.
        #[cfg(windows)]
        let framed = {
            let p = Mutex::new(self.proxy.clone());
            crate::frame::install(hwnd, move || {
                if let Ok(p) = p.lock() {
                    let _ = p.send_event(UserEvent::Frame);
                }
            })
        };
        #[cfg(not(windows))]
        let framed = false;
        trace(self.t0, "title bar themed");
        let Some(Loaded { te, fonts, settings, state }) = self.font_thread.take().and_then(|t| t.join().ok()).flatten()
        else {
            eprintln!("ffwd: no usable UI font found");
            self.exit_code = 1;
            el.exit();
            return;
        };
        trace(self.t0, "fonts ready");
        let sz = window.inner_size();
        self.size = (sz.width as usize, sz.height as usize);
        let mut ui = Ui::new(
            te,
            fonts,
            settings,
            state,
            self.first.clone().map(|p| (1, p)),
            self.sinks.clone(),
            self.size.0,
            self.size.1,
            window.scale_factor() as f32,
        );
        ui.remote = platform::is_remote_session();
        if ui.chrome == Chrome::Windows11 && !framed {
            // Without the frame hook the system caption stays; no second set of buttons.
            ui.chrome = Chrome::Native;
            ui.resize(self.size.0, self.size.1, window.scale_factor() as f32);
        }
        for (id, m) in self.pending.drain(..) {
            ui.on_tab_msg(id, m);
        }
        self.ui = Some(ui);
        trace(self.t0, "ui built");
        let mut gfx = Gfx { window, surface, hwnd };
        if let (Some(w), Some(h)) = (NonZeroU32::new(sz.width), NonZeroU32::new(sz.height)) {
            let _ = gfx.surface.resize(w, h);
        }
        if self.smoke {
            if !platform::cloak_and_show(hwnd) {
                eprintln!("ffwd: refusing --smoke without a cloaked window");
                self.exit_code = 1;
                el.exit();
                return;
            }
        } else {
            gfx.window.set_visible(true);
        }
        trace(self.t0, "shown");
        self.gfx = Some(gfx);
        self.after_input();
        self.render();
        self.start_update_check();
        self.smoke_check(el);
    }

    fn user_event(&mut self, el: &ActiveEventLoop, ev: UserEvent) {
        match ev {
            UserEvent::Msg(id, m) => match &mut self.ui {
                Some(ui) => {
                    let done_before = ui.r.log_done;
                    ui.on_tab_msg(id, m);
                    if !done_before && ui.r.log_done {
                        trace(self.t0, &format!("history done: {} commits", ui.r.commits.len()));
                    }
                }
                None => self.pending.push((id, m)),
            },
            UserEvent::Frame => {
                if let (Some(ui), Some(g)) = (&mut self.ui, &self.gfx) {
                    #[cfg(windows)]
                    let state = crate::frame::max_button_state();
                    #[cfg(not(windows))]
                    let state = (false, false);
                    ui.set_frame_state(state, g.window.is_maximized());
                }
            }
            UserEvent::UpdateFound(result) => match result {
                Ok(crate::update::Outcome::Found(c)) => {
                    if let Some(ui) = &mut self.ui {
                        ui.offer_update(&c.version);
                    }
                    self.update_offer = Some(c);
                }
                Ok(crate::update::Outcome::Off(why)) => {
                    if let Some(ui) = &mut self.ui {
                        ui.set_update_message(why);
                    }
                }
                Ok(crate::update::Outcome::UpToDate) => {
                    if let Some(ui) = &mut self.ui
                        && self.updates_forced
                    {
                        ui.set_update_message(format!("ffwd {} is the newest version.", env!("CARGO_PKG_VERSION")));
                    }
                }
                Ok(crate::update::Outcome::Skipped) => {}
                // A failed background check is not worth an alarm; it is
                // shown quietly and retried tomorrow.
                Err(e) => {
                    if let Some(ui) = &mut self.ui {
                        ui.set_update_message(format!("Update check failed: {e}"));
                    }
                }
            },
            UserEvent::UpdateFinished(result) => {
                self.update_installing = false;
                match result {
                    // The setup waits for ffwd to close, installs and restarts it.
                    Ok(()) => el.exit(),
                    Err(e) => {
                        if let Some(ui) = &mut self.ui {
                            ui.set_update_message(format!("Update failed: {e}"));
                        }
                    }
                }
            }
            UserEvent::SmokeTimeout => {
                eprintln!("ffwd: smoke timed out after 30 s");
                self.exit_code = 3;
                el.exit();
            }
        }
        self.after_input();
        self.request();
        self.smoke_check(el);
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, ev: WindowEvent) {
        match ev {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(sz) => {
                let new = (sz.width as usize, sz.height as usize);
                if new == self.size {
                    return;
                }
                self.size = new;
                if let (Some(g), Some(w), Some(h)) =
                    (&mut self.gfx, NonZeroU32::new(sz.width), NonZeroU32::new(sz.height))
                {
                    let _ = g.surface.resize(w, h);
                }
                if let (Some(ui), Some(g)) = (&mut self.ui, &self.gfx) {
                    ui.resize(new.0, new.1, g.window.scale_factor() as f32);
                    ui.set_remote(platform::is_remote_session());
                    let max_state = ui.max_state;
                    ui.set_frame_state(max_state, g.window.is_maximized());
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                if let Some(ui) = &mut self.ui {
                    ui.resize(self.size.0, self.size.1, scale_factor as f32);
                }
            }
            WindowEvent::RedrawRequested => {
                self.render();
                self.smoke_check(el);
                return;
            }
            // RDP connects and disconnects resize and refocus the window, so the
            // session type is re-read on those events (no timer, no polling).
            WindowEvent::Focused(focused) => {
                if let Some(ui) = &mut self.ui {
                    ui.set_active(focused);
                    if focused {
                        ui.set_remote(platform::is_remote_session());
                    }
                }
            }
            WindowEvent::ModifiersChanged(m) => self.mods = m.state(),
            WindowEvent::KeyboardInput { event, .. } => self.key(&event),
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x as i32, position.y as i32);
                if let Some(ui) = &mut self.ui {
                    ui.pointer(self.cursor.0, self.cursor.1);
                }
            }
            WindowEvent::CursorLeft { .. } => {
                if let Some(ui) = &mut self.ui {
                    ui.pointer_left();
                }
            }
            WindowEvent::MouseInput { state: ElementState::Released, button: MouseButton::Left, .. } => {
                let (x, y) = self.cursor;
                let action = self.ui.as_mut().and_then(|ui| {
                    ui.release(x, y);
                    ui.win_action.take()
                });
                match (action, &self.gfx) {
                    (Some(WinAction::Minimize), Some(g)) => g.window.set_minimized(true),
                    (Some(WinAction::Close), _) => el.exit(),
                    _ => {}
                }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                let (x, y) = self.cursor;
                if let Some(ui) = &mut self.ui {
                    ui.click(x, y);
                }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Middle, .. } => {
                let (x, y) = self.cursor;
                if let Some(ui) = &mut self.ui {
                    ui.middle_click(x, y);
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let (x, y) = self.cursor;
                if let Some(ui) = &mut self.ui {
                    let lines = match delta {
                        MouseScrollDelta::LineDelta(_, dy) => -dy * 3.0,
                        MouseScrollDelta::PixelDelta(p) => -(p.y as f32) / ui.lay.row_h.max(1) as f32,
                    };
                    ui.wheel(x, y, lines);
                }
            }
            _ => return,
        }
        self.after_input();
        self.request();
    }
}
