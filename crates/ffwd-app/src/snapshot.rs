//! Offscreen rendering of the real UI into a PNG. Same state, same paint
//! code as the window; no window, no desktop disturbance.

use crate::ui::{Focus, Key, Ui, load_fonts};
use crate::work::{self, Msg, Sink};
use crate::{Args, trace};
use ffwd_render::{Canvas, TextEngine};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

pub fn run(args: &Args, t0: Instant) -> i32 {
    let Some(out) = &args.snapshot else { return 2 };
    let (tx, rx) = mpsc::channel::<(u64, Msg)>();
    let tx = Arc::new(Mutex::new(tx));
    let sinks: crate::ui::SinkFactory = Arc::new(move |id| {
        let tx = tx.clone();
        let sink: Sink = Arc::new(move |m| {
            if let Ok(t) = tx.lock() {
                let _ = t.send((id, m));
            }
        });
        sink
    });
    let repo = match &args.repo {
        Some(r) => r.clone(),
        None => std::env::current_dir().unwrap_or_default(),
    };
    work::start(repo.clone(), &sinks(1));
    let mut te = TextEngine::new();
    let Some(fonts) = load_fonts(&mut te) else {
        eprintln!("ffwd: no usable UI font found");
        return 1;
    };
    let (w, h) = args.size;
    // Snapshots ignore the user's settings so they are reproducible;
    // `--layout below|beside` picks the layout.
    let layout = args.layout.as_deref().unwrap_or("below");
    let settings = crate::settings::Settings::parse(&format!("[ui]\nlayout = \"{layout}\"\n"));
    // Extra tabs (--tab) are shown, not loaded; the user's session is not read.
    let tabs: Vec<String> = args.tabs.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    let state =
        crate::settings::Settings::parse(&format!("[session]\ntabs = \"{}\"\n", tabs.join("|").replace('\\', "\\\\")));
    let mut ui = Ui::new(te, fonts, settings, state, Some((1, repo.clone())), sinks, w, h, args.scale.unwrap_or(1.0));
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut selected = false;
    let mut have_refs = false;
    let mut have_status = false;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let Ok((id, m)) = rx.recv_timeout(left) else { break };
        have_refs |= matches!(m, Msg::Refs(_));
        have_status |= matches!(m, Msg::Status(_));
        let repo_err = matches!(m, Msg::Repo(Err(_)));
        ui.on_tab_msg(id, m);
        if repo_err {
            break;
        }
        if ui.r.log_done && have_refs && have_status && !selected {
            if let Some(n) = args.select {
                ui.select(n);
            }
            selected = true;
        }
        let diff_ready = ui.r.diff.is_some() || ui.r.details.as_ref().is_some_and(|d| d.files.is_empty());
        if selected && ui.r.details.is_some() && diff_ready {
            break;
        }
    }
    trace(t0, "data ready");
    match args.focus.as_deref() {
        Some("files") => ui.key(Key::Tab),
        Some("diff") => {
            ui.key(Key::Tab);
            ui.key(Key::Tab);
        }
        _ => {}
    }
    if ui.focus == Focus::Files {
        // keep the first file selected; the diff is already loaded
    }
    if let Some(q) = &args.palette {
        ui.open_palette_with(q);
    }
    if let Some(name) = &args.side {
        ui.snapshot_side(name);
    }
    if args.changes {
        // The view loads its state on workers; wait (bounded) for the
        // list and the first diff.
        ui.enter_changes();
        let until = Instant::now() + Duration::from_secs(10);
        let mut have_list = false;
        while let Ok((id, m)) = rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
            have_list |= matches!(m, Msg::Changes(_));
            let diff = matches!(m, Msg::ChangesDiff { .. });
            ui.on_tab_msg(id, m);
            if have_list && (diff || ui.r.changes.status.as_ref().is_some_and(|s| s.entries.is_empty())) {
                break;
            }
        }
    }
    if args.settings {
        ui.open_settings();
    }
    if let Some(p) = args.settings_page {
        ui.open_settings_page(p);
    }
    if args.agents_demo {
        ui.snapshot_agents_demo();
    }
    if args.scope_demo {
        ui.snapshot_repo_scope_demo();
    }
    if let Some(q) = &args.picker {
        // Let the repository scan arrive (bounded wait).
        let until = Instant::now() + Duration::from_secs(5);
        while let Ok((id, m)) = rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
            let done = matches!(m, Msg::Repos(_));
            ui.on_tab_msg(id, m);
            if done {
                break;
            }
        }
        ui.open_repo_picker();
        for c in q.chars() {
            ui.key(Key::Char(c));
        }
    }
    let mut px = vec![0u32; w * h];
    let mut cv = Canvas::new(&mut px, w, h);
    if let Some((x, y)) = args.hover {
        // Hit areas come from a paint; then the pointer rests at (x, y).
        ui.paint(&mut cv, true);
        ui.pointer(x, y);
    }
    let t = Instant::now();
    ui.paint(&mut cv, true);
    let full_ms = t.elapsed().as_secs_f64() * 1000.0;
    trace(t0, &format!("full paint {full_ms:.2} ms"));
    // One selection step, to report what a key press costs.
    ui.key(Key::Down);
    let mut scratch = px.clone();
    let mut cv2 = Canvas::new(&mut scratch, w, h);
    let step = ui.paint(&mut cv2, false);
    eprintln!(
        "snapshot: {} commits, log {:.0} ms (first rows {:.0} ms), full paint {:.2} ms; key Down repaints {} px ({:.1} % of window) in {:.2} ms",
        ui.r.commits.len(),
        ui.r.log_ms,
        ui.r.first_batch_ms.unwrap_or(0.0),
        full_ms,
        ui.stats.last_damage_px,
        100.0 * ui.stats.last_damage_px as f64 / (w * h) as f64,
        ui.stats.last_paint_ms,
    );
    let _ = step;
    match write_png(out, &px, w, h) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("ffwd: could not write {}: {e}", out.display());
            1
        }
    }
}

fn write_png(path: &std::path::Path, px: &[u32], w: usize, h: usize) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut wr = enc.write_header().map_err(|e| e.to_string())?;
    let mut rgb = Vec::with_capacity(w * h * 3);
    for &p in px {
        rgb.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8]);
    }
    wr.write_image_data(&rgb).map_err(|e| e.to_string())
}

pub fn write_rgba_png(path: &std::path::Path, rgba: &[u8], w: usize, h: usize) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut wr = enc.write_header().map_err(|e| e.to_string())?;
    wr.write_image_data(rgba).map_err(|e| e.to_string())
}
