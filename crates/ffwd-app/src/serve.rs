//! Persistent headless UI server (`ffwd --serve [port]`).
//!
//! Keeps parsed fonts, the synthetic demo repository, and the pixel buffer hot
//! in memory on a loopback TCP socket (`127.0.0.1`). Agents and scripts can
//! reload design tokens (`tokens <path>`), switch screens/themes, drive real
//! input (`hover`, `click`, `key`), run `--audit`, and render fresh PNGs in
//! ~15–25 ms with zero process startup, zero font parsing, and zero git spawns.

use crate::Args;
use crate::audit;
use crate::demo;
use crate::snapshot::{build_demo_ui, write_png_fast};
use crate::theme;
use crate::ui::{DetailsLayout, Key, Ui};
use ffwd_render::Canvas;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::time::Instant;

struct State {
    ui: Ui,
    px: Vec<u32>,
    screen: String,
    is_sheet: bool,
}

impl State {
    fn render_current(&mut self) -> f64 {
        let need = self.ui.w * self.ui.h;
        if self.px.len() != need {
            self.px.resize(need, 0);
        }
        self.ui.te.reset_ellipsis_count();
        let t = Instant::now();
        let mut cv = Canvas::new(&mut self.px, self.ui.w, self.ui.h);
        if self.is_sheet {
            demo::paint_sheet(&mut self.ui, &mut cv);
        } else {
            self.ui.paint(&mut cv, true);
        }
        t.elapsed().as_secs_f64() * 1000.0
    }
}

pub fn run(args: &Args, t0: Instant) -> i32 {
    let port = args.serve_port.unwrap_or(0);
    if let Some(tokens) = &args.tokens
        && let Err(e) = theme::load_tokens_file(tokens)
    {
        eprintln!("ffwd: {e}");
        return 1;
    }
    let mut ui = match build_demo_ui(args) {
        Ok(u) => u,
        Err(e) => {
            eprintln!("ffwd: {e}");
            return 1;
        }
    };
    if let Some(t) = &args.theme {
        ui.settings.set_string("ui.theme", t);
        ui.apply_theme_now();
    }
    let px = vec![0u32; ui.w * ui.h];
    let mut state = State { ui, px, screen: "history".into(), is_sheet: false };
    // Warm up the glyph cache once on startup so the first `snap` is instant.
    state.render_current();

    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("ffwd: could not bind 127.0.0.1:{port}: {e}");
            return 1;
        }
    };
    let bound_port = listener.local_addr().map_or(port, |a| a.port());
    let startup_ms = t0.elapsed().as_secs_f64() * 1000.0;
    println!("{{\"ready\":true,\"port\":{bound_port},\"startup_ms\":{startup_ms:.1}}}");
    let _ = std::io::stdout().flush();

    for stream in listener.incoming().flatten() {
        let Ok(mut writer) = stream.try_clone() else { continue };
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let cmd = line.trim();
            if cmd.is_empty() || cmd.starts_with('#') {
                continue;
            }
            let (resp, quit) = handle(&mut state, cmd);
            let _ = writeln!(writer, "{resp}");
            let _ = writer.flush();
            if quit {
                return 0;
            }
        }
    }
    0
}

fn ok_json(extra: &str) -> String {
    if extra.is_empty() { "{\"ok\":true}".into() } else { format!("{{\"ok\":true,{extra}}}") }
}

fn err_json(msg: &str) -> String {
    let esc = msg.replace('\\', "\\\\").replace('"', "\\\"");
    format!("{{\"ok\":false,\"error\":\"{esc}\"}}")
}

fn handle(st: &mut State, line: &str) -> (String, bool) {
    let mut parts = line.splitn(2, char::is_whitespace);
    let verb = parts.next().unwrap_or("");
    let rest = parts.next().unwrap_or("").trim();

    match verb {
        "ping" => (ok_json(""), false),
        "quit" | "exit" => (ok_json("\"bye\":true"), true),
        "screen" => match demo::apply_screen(&mut st.ui, rest) {
            Ok(is_sheet) => {
                st.screen = rest.to_string();
                st.is_sheet = is_sheet;
                (ok_json(&format!("\"screen\":\"{}\"", st.screen)), false)
            }
            Err(e) => (err_json(&e), false),
        },
        "theme" => match rest {
            "dark" | "light" => {
                st.ui.settings.set_string("ui.theme", rest);
                st.ui.apply_theme_now();
                (ok_json(&format!("\"theme\":\"{rest}\"")), false)
            }
            _ => (err_json("theme expects `dark` or `light`"), false),
        },
        "sidebar" => {
            st.ui.set_side_style(rest);
            (ok_json(&format!("\"sidebar\":\"{rest}\"")), false)
        }
        "layout" => {
            st.ui.set_layout(DetailsLayout::from_setting(Some(rest)));
            (ok_json(&format!("\"layout\":\"{rest}\"")), false)
        }
        "size" => {
            let mut it = rest.split_whitespace();
            let Some((w, h)) = it.next().and_then(|s| s.split_once('x')) else {
                return (err_json("size expects `WxH [scale]`"), false);
            };
            let (Ok(w), Ok(h)) = (w.parse::<usize>(), h.parse::<usize>()) else {
                return (err_json("bad width or height"), false);
            };
            let scale = it.next().and_then(|s| s.parse::<f32>().ok()).unwrap_or(st.ui.s);
            if w == 0 || h == 0 || w > 8192 || h > 8192 {
                return (err_json("size out of range (1..=8192)"), false);
            }
            st.ui.resize(w, h, scale);
            (ok_json(&format!("\"size\":[{w},{h}],\"scale\":{scale:.2}")), false)
        }
        "tokens" => {
            let path = PathBuf::from(rest.trim_matches('"'));
            let t = Instant::now();
            match theme::load_tokens_file(&path) {
                Ok(n) => {
                    st.ui.apply_tokens_now();
                    let ms = t.elapsed().as_secs_f64() * 1000.0;
                    (ok_json(&format!("\"applied\":{n},\"reload_ms\":{ms:.2}")), false)
                }
                Err(e) => (err_json(&e), false),
            }
        }
        "tokens_reset" => {
            theme::reset_tokens();
            st.ui.apply_tokens_now();
            (ok_json("\"reset\":true"), false)
        }
        "hover" => {
            let mut it = rest.split_whitespace();
            let (Some(x), Some(y)) =
                (it.next().and_then(|v| v.parse::<i32>().ok()), it.next().and_then(|v| v.parse::<i32>().ok()))
            else {
                return (err_json("hover expects `X Y`"), false);
            };
            // Ensure hit rects exist before hovering.
            st.render_current();
            st.ui.pointer(x, y);
            (ok_json(&format!("\"hover\":[{x},{y}]")), false)
        }
        "click" => {
            let mut it = rest.split_whitespace();
            let (Some(x), Some(y)) =
                (it.next().and_then(|v| v.parse::<i32>().ok()), it.next().and_then(|v| v.parse::<i32>().ok()))
            else {
                return (err_json("click expects `X Y`"), false);
            };
            st.render_current();
            st.ui.click(x, y);
            (ok_json(&format!("\"click\":[{x},{y}]")), false)
        }
        "key" => match parse_key(rest) {
            Some(k) => {
                st.ui.key(k);
                (ok_json(&format!("\"key\":\"{rest}\"")), false)
            }
            None => (err_json("unknown key name"), false),
        },
        "snap" => {
            let out = PathBuf::from(rest.trim_matches('"'));
            if rest.is_empty() {
                return (err_json("snap expects `<out.png>`"), false);
            }
            if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
                let _ = std::fs::create_dir_all(parent);
            }
            let total_t = Instant::now();
            let paint_ms = st.render_current();
            let png_t = Instant::now();
            let res = write_png_fast(&out, &st.px, st.ui.w, st.ui.h);
            let png_ms = png_t.elapsed().as_secs_f64() * 1000.0;
            let total_ms = total_t.elapsed().as_secs_f64() * 1000.0;
            match res {
                Ok(()) => (
                    ok_json(&format!(
                        "\"screen\":\"{}\",\"theme\":\"{}\",\"paint_ms\":{paint_ms:.2},\"png_ms\":{png_ms:.2},\"total_ms\":{total_ms:.2},\"ellipsis\":{}",
                        st.screen,
                        if theme::is_light() { "light" } else { "dark" },
                        st.ui.te.ellipsis_count()
                    )),
                    false,
                ),
                Err(e) => (err_json(&e), false),
            }
        }
        "batch" => {
            let mut it = rest.split_whitespace();
            let Some(dir_str) = it.next() else {
                return (err_json("batch expects `<dir> [only_filter]`"), false);
            };
            let only = it.next();
            let dir = Path::new(dir_str.trim_matches('"'));
            if let Err(e) = std::fs::create_dir_all(dir) {
                return (err_json(&format!("could not create dir: {e}")), false);
            }
            let t = Instant::now();
            let mut count = 0usize;
            for &scr in demo::SCREENS {
                if only.is_some_and(|f| !scr.contains(f)) {
                    continue;
                }
                let Ok(is_sheet) = demo::apply_screen(&mut st.ui, scr) else { continue };
                st.screen = scr.to_string();
                st.is_sheet = is_sheet;
                for &th in &["dark", "light"] {
                    st.ui.settings.set_string("ui.theme", th);
                    st.ui.apply_theme_now();
                    st.render_current();
                    let p = dir.join(format!("{scr}-{th}.png"));
                    if let Err(e) = write_png_fast(&p, &st.px, st.ui.w, st.ui.h) {
                        return (err_json(&e), false);
                    }
                    count += 1;
                }
            }
            let total_ms = t.elapsed().as_secs_f64() * 1000.0;
            (ok_json(&format!("\"count\":{count},\"total_ms\":{total_ms:.1}")), false)
        }
        "audit" => {
            let paint_ms = st.render_current();
            let rep = audit::run(&mut st.ui, &st.screen, &st.px, paint_ms, !st.is_sheet);
            (rep.to_json(), false)
        }
        other => (
            err_json(&format!(
                "unknown command `{other}`; try ping|screen|theme|sidebar|layout|size|tokens|tokens_reset|hover|click|key|snap|batch|audit|quit"
            )),
            false,
        ),
    }
}

fn parse_key(s: &str) -> Option<Key> {
    match s.to_ascii_lowercase().as_str() {
        "up" => Some(Key::Up),
        "down" => Some(Key::Down),
        "left" => Some(Key::Left),
        "right" => Some(Key::Right),
        "pageup" => Some(Key::PageUp),
        "pagedown" => Some(Key::PageDown),
        "home" => Some(Key::Home),
        "end" => Some(Key::End),
        "tab" => Some(Key::Tab),
        "backtab" => Some(Key::BackTab),
        "enter" => Some(Key::Enter),
        "esc" | "escape" => Some(Key::Esc),
        "backspace" => Some(Key::Backspace),
        "delete" => Some(Key::Delete),
        "palette" | "ctrl+k" => Some(Key::Palette),
        "space" => Some(Key::Char(' ')),
        other if other.len() == 1 => other.chars().next().map(Key::Char),
        other => {
            let c = other.strip_prefix("ctrl+")?.chars().next()?;
            Some(Key::Ctrl(c))
        }
    }
}
