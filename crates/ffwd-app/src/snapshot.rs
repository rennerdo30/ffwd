//! Offscreen rendering of the real UI into PNGs: single repo snapshots
//! (`--snapshot`), zero-git synthetic snapshots (`--demo`), single-process
//! multi-screen batches (`--snapshot-ui <dir>`), and programmatic UI audits
//! (`--audit`). Same state, same paint code as the window; no desktop impact.

use crate::audit;
use crate::demo;
use crate::theme;
use crate::ui::{Focus, Key, Ui, load_fonts};
use crate::work::{self, Msg, Sink};
use crate::{Args, trace};
use ffwd_render::{Canvas, TextEngine};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

/// Builds a hot `Ui` pre-populated with the in-memory synthetic demo repo
/// (zero `git.exe` spawns, zero disk reads beyond the 3 font files).
pub fn build_demo_ui(args: &Args) -> Result<Ui, String> {
    let mut te = TextEngine::new();
    let fonts = load_fonts(&mut te).ok_or_else(|| "no usable UI font found".to_string())?;
    let (w, h) = args.size;
    let layout = args.layout.as_deref().unwrap_or("below");
    let settings = crate::settings::Settings::parse(&format!("[ui]\nlayout = \"{layout}\"\n"));
    let state = crate::settings::Settings::parse("");
    let sinks: crate::ui::SinkFactory = Arc::new(|_| Arc::new(|_| {}));
    let mut ui = Ui::new(
        te,
        fonts,
        settings,
        state,
        Some((1, PathBuf::from("/demo/ffwd"))),
        sinks,
        w,
        h,
        args.scale.unwrap_or(1.25),
    );
    if let Some(st) = &args.sidebar {
        ui.set_side_style(st);
    }
    demo::populate(&mut ui);
    Ok(ui)
}

/// Single-process batch capture of all built-in demo screens (`--snapshot-ui <dir>`).
pub fn run_batch(args: &Args, t0: Instant) -> i32 {
    let Some(dir) = &args.snapshot_ui else { return 2 };
    if let Some(tokens) = &args.tokens
        && let Err(e) = theme::load_tokens_file(tokens)
    {
        eprintln!("ffwd: {e}");
        return 1;
    }
    if let Err(e) = std::fs::create_dir_all(dir) {
        eprintln!("ffwd: could not create {}: {e}", dir.display());
        return 1;
    }
    let mut ui = match build_demo_ui(args) {
        Ok(u) => u,
        Err(e) => {
            eprintln!("ffwd: {e}");
            return 1;
        }
    };
    let fonts_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let themes: &[&str] = match args.themes.as_deref() {
        Some("dark") => &["dark"],
        Some("light") => &["light"],
        _ => &["dark", "light"],
    };
    let mut px = vec![0u32; ui.w * ui.h];
    let mut audits: Vec<String> = Vec::new();
    let mut count = 0usize;
    let mut paint_total_ms = 0.0f64;
    let mut png_total_ms = 0.0f64;
    let mut audit_failed = false;

    for &screen in demo::SCREENS {
        if let Some(only) = &args.only
            && !screen.contains(only.as_str())
        {
            continue;
        }
        let is_sheet = match demo::apply_screen(&mut ui, screen) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("ffwd: {e}");
                return 1;
            }
        };
        for &th in themes {
            ui.settings.set_string("ui.theme", th);
            ui.apply_theme_now();
            ui.te.reset_ellipsis_count();
            let tp = Instant::now();
            {
                let mut cv = Canvas::new(&mut px, ui.w, ui.h);
                if is_sheet {
                    demo::paint_sheet(&mut ui, &mut cv);
                } else {
                    ui.paint(&mut cv, true);
                }
            }
            let pms = tp.elapsed().as_secs_f64() * 1000.0;
            paint_total_ms += pms;

            if args.audit.is_some() {
                let rep = audit::run(&mut ui, screen, &px, pms, !is_sheet && screen == "history");
                if !rep.ok() {
                    audit_failed = true;
                }
                audits.push(rep.to_json());
            }

            let tw = Instant::now();
            let out = dir.join(format!("{screen}-{th}.png"));
            if let Err(e) = write_png_fast(&out, &px, ui.w, ui.h) {
                eprintln!("ffwd: could not write {}: {e}", out.display());
                return 1;
            }
            png_total_ms += tw.elapsed().as_secs_f64() * 1000.0;
            count += 1;
        }
    }

    if let Some(audit_out) = &args.audit {
        let json = format!("[{}]\n", audits.join(","));
        if audit_out.as_os_str().is_empty() {
            println!("{json}");
        } else if let Err(e) = std::fs::write(audit_out, &json) {
            eprintln!("ffwd: could not write audit {}: {e}", audit_out.display());
            return 1;
        }
    }

    let total_ms = t0.elapsed().as_secs_f64() * 1000.0;
    eprintln!(
        "snapshot-ui: {} screens in {:.1} ms (setup {:.1} ms, paint {:.1} ms, png {:.1} ms, {:.1} ms/screen)",
        count,
        total_ms,
        fonts_ms,
        paint_total_ms,
        png_total_ms,
        if count > 0 { total_ms / count as f64 } else { 0.0 }
    );
    if audit_failed { 1 } else { 0 }
}

pub fn run(args: &Args, t0: Instant) -> i32 {
    let Some(out) = &args.snapshot else { return 2 };
    if let Some(tokens) = &args.tokens
        && let Err(e) = theme::load_tokens_file(tokens)
    {
        eprintln!("ffwd: {e}");
        return 1;
    }
    if args.demo {
        return run_demo_single(args, out, t0);
    }
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
    if let Some(st) = &args.sidebar {
        ui.set_side_style(st);
    }
    if let Some(t) = &args.theme {
        ui.settings.set_string("ui.theme", t);
        ui.apply_theme_now();
    }
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
    if args.diff_ignore_ws {
        ui.diff_opts.ignore_whitespace = true;
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
    if let Some(path) = &args.files {
        // The tree, then the file and its tab, all from workers (bounded).
        ui.enter_files();
        let until = Instant::now() + Duration::from_secs(15);
        let mut opened = false;
        while let Ok((id, m)) = rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
            let tree = matches!(m, Msg::FilesTree(_));
            let done = matches!(m, Msg::FileContent { .. } | Msg::FileBlame { .. } | Msg::FileHistory { .. });
            ui.on_tab_msg(id, m);
            if tree && !opened {
                if path.is_empty() || !ui.reveal_file(path) {
                    break;
                }
                opened = true;
                if let Some(t) = &args.files_tab {
                    ui.snapshot_files_tab(t);
                }
            }
            if done && ui.files_ready() {
                break;
            }
        }
    }
    if args.settings {
        ui.open_settings();
    }
    if args.menu {
        ui.snapshot_menu();
    }
    if let Some(keys) = &args.pick {
        ui.snapshot_pick(keys);
    }
    if args.prompt {
        ui.snapshot_prompt();
    }
    if args.stash_dialog {
        ui.open_stash_dialog();
    }
    if args.ai_preview {
        // Only the preview: it is built on a worker and nothing is sent.
        ui.snapshot_ai_preview();
        let until = Instant::now() + Duration::from_secs(10);
        while let Ok((id, m)) = rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
            let done = matches!(m, Msg::AiPrepared { .. });
            ui.on_tab_msg(id, m);
            if done {
                break;
            }
        }
    }
    if args.activity {
        // Both fail without changing anything (a missing branch and tag).
        ui.snapshot_activity();
        let until = Instant::now() + Duration::from_secs(20);
        let mut n = 0;
        while let Ok((id, m)) = rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
            n += usize::from(matches!(m, Msg::Activity(_)));
            ui.on_tab_msg(id, m);
            if n >= 2 {
                break;
            }
        }
        ui.enter_activity();
    }
    if let Some(row) = args.rebase_from {
        // Only the plan: it is read on a worker and changes nothing.
        ui.snapshot_rebase(row);
        let until = Instant::now() + Duration::from_secs(10);
        while let Ok((id, m)) = rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
            let done = matches!(m, Msg::RebasePlan(_));
            ui.on_tab_msg(id, m);
            if done {
                break;
            }
        }
        ui.snapshot_rebase_demo();
    }
    if let Some(s) = args.code_size {
        ui.set_code_size(s);
    }
    if args.diff_wrap {
        ui.diff_wrap = true;
    }
    if args.diff_ws {
        ui.diff_show_ws = true;
    }
    if args.diff_split {
        ui.diff_split = true;
    }
    if let Some(n) = args.file {
        ui.snapshot_file(n);
        let until = Instant::now() + Duration::from_secs(10);
        while let Ok((id, m)) = rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
            let diff = matches!(m, Msg::Diff { .. });
            ui.on_tab_msg(id, m);
            if diff && ui.r.diff.is_some() {
                break;
            }
        }
    }
    if let Some(q) = args.find.as_ref().and_then(|q| q.strip_prefix("diff:")) {
        // Find in the shown diff (FF-34).
        ui.focus = crate::ui::Focus::Diff;
        ui.key(Key::Ctrl('f'));
        for c in q.chars() {
            ui.key(Key::Char(c));
        }
    } else if let Some(q) = &args.find {
        // `content:` and `path:` pick that field (the chips after Message,
        // Author and SHA) and start with Enter.
        let (tabs, q) = match (q.strip_prefix("content:"), q.strip_prefix("path:")) {
            (Some(q), _) => (3, q),
            (_, Some(q)) => (4, q),
            _ => (0, q.as_str()),
        };
        ui.key(Key::Ctrl('f'));
        for _ in 0..tabs {
            ui.key(Key::Tab);
        }
        for c in q.chars() {
            ui.key(Key::Char(c));
        }
        if tabs > 0 {
            ui.key(Key::Enter);
        }
        let until = Instant::now() + Duration::from_secs(10);
        let mut searched = false;
        while let Ok((id, m)) = rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
            searched |= matches!(m, Msg::SearchBatch { done: true, .. });
            ui.on_tab_msg(id, m);
            // Then the selected match's details and diff.
            if searched && ui.r.diff.is_some() {
                break;
            }
        }
    }
    if let Some(p) = args.settings_page {
        ui.open_settings_page(p);
    }
    if let Some(on) = args.agents_page {
        if on {
            ui.settings.set_string("agents.enabled", "true");
            ui.settings.set_string("agents.preset", "claude");
        }
        ui.open_agents();
    }
    if let Some(s) = &args.settings_section {
        ui.open_settings_named(s);
    }
    if args.agents_demo {
        ui.snapshot_agents_demo();
    }
    if args.scope_demo {
        ui.snapshot_repo_scope_demo();
    }
    if args.access_demo {
        ui.snapshot_access_demo();
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
    ui.te.reset_ellipsis_count();
    let t = Instant::now();
    ui.paint(&mut cv, true);
    let full_ms = t.elapsed().as_secs_f64() * 1000.0;
    trace(t0, &format!("full paint {full_ms:.2} ms"));
    if let Some(audit_out) = &args.audit {
        let rep = audit::run(&mut ui, "snapshot", &px, full_ms, true);
        let json = rep.to_json();
        if audit_out.as_os_str().is_empty() {
            println!("{json}");
        } else {
            let _ = std::fs::write(audit_out, format!("{json}\n"));
        }
    }
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
    match write_png_fast(out, &px, w, h) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("ffwd: could not write {}: {e}", out.display());
            1
        }
    }
}

/// Fast single-screen synthetic snapshot (`--demo --snapshot out.png`).
fn run_demo_single(args: &Args, out: &std::path::Path, t0: Instant) -> i32 {
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
    let screen = if let Some(s) = &args.only {
        s.as_str()
    } else if args.changes {
        "changes"
    } else if args.settings {
        "settings"
    } else if args.agents_demo || args.agents_page.is_some() {
        "agents"
    } else if args.diff_split {
        "diff-split"
    } else if args.palette.is_some() {
        "palette"
    } else if args.files.is_some() {
        "files"
    } else if args.activity {
        "activity"
    } else {
        "history"
    };
    let is_sheet = match demo::apply_screen(&mut ui, screen) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("ffwd: {e}");
            return 1;
        }
    };
    if let Some(s) = &args.settings_section {
        ui.open_settings_named(s);
    }
    if let Some(q) = &args.palette {
        ui.open_palette_with(q);
    }
    if let Some(s) = args.code_size {
        ui.set_code_size(s);
    }
    if args.diff_wrap {
        ui.diff_wrap = true;
    }
    if args.diff_ws {
        ui.diff_show_ws = true;
    }
    let mut px = vec![0u32; ui.w * ui.h];
    if let Some((x, y)) = args.hover {
        let mut cv = Canvas::new(&mut px, ui.w, ui.h);
        ui.paint(&mut cv, true);
        ui.pointer(x, y);
    }
    ui.te.reset_ellipsis_count();
    let t = Instant::now();
    {
        let mut cv = Canvas::new(&mut px, ui.w, ui.h);
        if is_sheet {
            demo::paint_sheet(&mut ui, &mut cv);
        } else {
            ui.paint(&mut cv, true);
        }
    }
    let full_ms = t.elapsed().as_secs_f64() * 1000.0;
    let mut audit_failed = false;
    if let Some(audit_out) = &args.audit {
        let rep = audit::run(&mut ui, screen, &px, full_ms, !is_sheet && screen == "history");
        audit_failed = !rep.ok();
        let json = rep.to_json();
        if audit_out.as_os_str().is_empty() {
            println!("{json}");
        } else if let Err(e) = std::fs::write(audit_out, format!("{json}\n")) {
            eprintln!("ffwd: could not write audit {}: {e}", audit_out.display());
            return 1;
        }
    }
    let tw = Instant::now();
    if let Err(e) = write_png_fast(out, &px, ui.w, ui.h) {
        eprintln!("ffwd: could not write {}: {e}", out.display());
        return 1;
    }
    let png_ms = tw.elapsed().as_secs_f64() * 1000.0;
    let total_ms = t0.elapsed().as_secs_f64() * 1000.0;
    eprintln!(
        "demo snapshot ({screen}): total {total_ms:.1} ms (paint {full_ms:.2} ms, png {png_ms:.1} ms, ellipsis {})",
        ui.te.ellipsis_count()
    );
    if audit_failed { 1 } else { 0 }
}

/// Fast RGB PNG writer (`Compression::Fast` + `FilterType::Sub`): 4–6× faster
/// than default zlib compression for UI screenshots while keeping small files.
pub fn write_png_fast(path: &std::path::Path, px: &[u32], w: usize, h: usize) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::with_capacity(1 << 16, file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::Fastest);
    let mut wr = enc.write_header().map_err(|e| e.to_string())?;
    let mut rgb = vec![0u8; w * h * 3];
    for (dst, &p) in rgb.as_chunks_mut::<3>().0.iter_mut().zip(px.iter()) {
        dst[0] = (p >> 16) as u8;
        dst[1] = (p >> 8) as u8;
        dst[2] = p as u8;
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
