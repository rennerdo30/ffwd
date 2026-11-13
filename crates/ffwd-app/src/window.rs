//! The window: winit for events, softbuffer for damage-only CPU presents.
//! No frame loop: the event loop waits, and a redraw is requested only when
//! the UI recorded damage.

use crate::platform;
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
    Msg(Msg),
    SmokeTimeout,
    /// The Windows frame changed the maximise button or window state.
    #[cfg_attr(not(windows), allow(dead_code))]
    Frame,
    UpdateFound(Result<crate::update::Outcome, String>),
    UpdateFinished(Result<(), String>),
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
    font_thread: Option<std::thread::JoinHandle<Option<(TextEngine, crate::ui::Fonts, crate::settings::Settings)>>>,
    sink: Sink,
    pending: Vec<Msg>,
    repo_name: String,
    repo: std::path::PathBuf,
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
    let proxy = Mutex::new(event_loop.create_proxy());
    let sink: Sink = Arc::new(move |m| {
        if let Ok(p) = proxy.lock() {
            let _ = p.send_event(UserEvent::Msg(m));
        }
    });
    // Git work starts before the window exists: first frame first, data behind it.
    work::start(args.repo.clone(), &sink);
    if args.smoke {
        let p = Mutex::new(event_loop.create_proxy());
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(30));
            if let Ok(p) = p.lock() {
                let _ = p.send_event(UserEvent::SmokeTimeout);
            }
        });
    }
    let font_thread = Some(std::thread::spawn(move || {
        let mut te = TextEngine::new();
        // Settings are read here too, so file I/O stays off the UI thread.
        let settings = crate::settings::Settings::load();
        let f = load_fonts(&mut te).map(|f| (te, f, settings));
        trace(t0, "fonts parsed (worker)");
        f
    }));
    let repo_name = args.repo.file_name().map_or("repository".into(), |n| n.to_string_lossy().into_owned());
    let mut app = App {
        t0,
        smoke: args.smoke,
        ui: None,
        font_thread,
        sink,
        pending: Vec::new(),
        repo_name,
        repo: args.repo.clone(),
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
        let diff_ready = ui.diff.is_some() || ui.details.as_ref().is_some_and(|d| d.files.is_empty());
        if ui.log_done && ui.details.is_some() && diff_ready && ui.damage.is_empty() {
            self.smoke_reported = true;
            println!(
                "{{\"first_frame_ms\":{:.1},\"commits\":{},\"log_ms\":{:.1},\"first_rows_ms\":{:.1},\"presents\":{},\"remote\":{}}}",
                ui.stats.first_frame_ms.unwrap_or(-1.0),
                ui.commits.len(),
                ui.log_ms,
                ui.first_batch_ms.unwrap_or(-1.0),
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
                NamedKey::ArrowDown => Some(Key::Down),
                NamedKey::PageUp => Some(Key::PageUp),
                NamedKey::PageDown => Some(Key::PageDown),
                NamedKey::Home => Some(Key::Home),
                NamedKey::End => Some(Key::End),
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
            WKey::Character(_) if ctrl => None,
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
        if ctrl
            && self.mods.shift_key()
            && matches!(&ev.logical_key, WKey::Character(s) if s.eq_ignore_ascii_case("u"))
            && let Some(ui) = &mut self.ui
        {
            ui.request_update_install();
        }
        self.process_update_request();
    }

    /// One update check per run, after the first frame; never in agent runs.
    fn start_update_check(&mut self) {
        if self.updates_started || self.smoke {
            return;
        }
        self.updates_started = true;
        let Some(policy) = self.ui.as_ref().map(|ui| ui.update_policy()) else { return };
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
            .with_title(format!("{} · ffwd", self.repo_name))
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
        let Some((te, fonts, settings)) = self.font_thread.take().and_then(|t| t.join().ok()).flatten() else {
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
            &self.repo,
            self.sink.clone(),
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
        for m in self.pending.drain(..) {
            ui.on_msg(m);
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
        self.render();
        self.start_update_check();
        self.smoke_check(el);
    }

    fn user_event(&mut self, el: &ActiveEventLoop, ev: UserEvent) {
        match ev {
            UserEvent::Msg(m) => match &mut self.ui {
                Some(ui) => {
                    let done_before = ui.log_done;
                    ui.on_msg(m);
                    if !done_before && ui.log_done {
                        trace(self.t0, &format!("history done: {} commits", ui.commits.len()));
                    }
                }
                None => self.pending.push(m),
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
                Ok(_) => {}
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
        self.request();
    }
}
