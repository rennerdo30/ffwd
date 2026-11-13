//! ffwd: a fast, keyboard-first Git client.
//!
//!   ffwd [repo]                       open a window
//!   ffwd [repo] --snapshot out.png    render the real UI offscreen, no window
//!       [--size 1600x1000] [--scale 1.25] [--select N] [--focus files|diff] [--palette "query"]
//!   ffwd [repo] --smoke               hidden, cloaked window; prints startup marks and exits

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod fmt;
#[cfg(windows)]
mod frame;
mod highlight;
mod icon;
mod platform;
mod settings;
mod snapshot;
mod text_area;
mod text_field;
mod theme;
mod ui;
mod update;
mod window;
mod work;

use std::path::PathBuf;
use std::time::Instant;

pub struct Args {
    /// The repository named on the command line; without one, ffwd
    /// restores the tabs of the last session.
    pub repo: Option<PathBuf>,
    /// Extra tabs for snapshots (`--tab <path>`, repeatable).
    pub tabs: Vec<PathBuf>,
    pub snapshot: Option<PathBuf>,
    pub smoke: bool,
    pub size: (usize, usize),
    pub scale: Option<f32>,
    pub select: Option<usize>,
    pub focus: Option<String>,
    pub palette: Option<String>,
    pub layout: Option<String>,
    /// Snapshot with the repository picker open on this query.
    pub picker: Option<String>,
    /// Snapshot with the settings view open.
    pub settings: bool,
    /// Snapshot: click this sidebar ref (`main`, `t3/x`) first.
    pub side: Option<String>,
    /// Snapshot with the settings view on this page (0 general, 1 agents).
    pub settings_page: Option<usize>,
    /// Snapshot with the agents panel open, showing example runs.
    pub agents_demo: bool,
    /// Snapshot of Settings → Agents for one repository (example values).
    pub scope_demo: bool,
    /// Snapshot with the pointer resting here (hover states).
    pub hover: Option<(i32, i32)>,
    /// Snapshot of the Local Changes view.
    pub changes: bool,
    /// Snapshot with the selected commit's context menu open.
    pub menu: bool,
    /// Snapshot with the "create branch" prompt open.
    pub prompt: bool,
    /// Snapshot with the find bar searching messages for this text.
    pub find: Option<String>,
    /// Snapshot with the diff side by side.
    pub diff_split: bool,
    /// Snapshot with this file of the selected commit shown.
    pub file: Option<usize>,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        repo: None,
        tabs: Vec::new(),
        snapshot: None,
        smoke: false,
        size: (1600, 1000),
        scale: None,
        select: None,
        focus: None,
        palette: None,
        layout: None,
        picker: None,
        settings: false,
        side: None,
        settings_page: None,
        agents_demo: false,
        scope_demo: false,
        hover: None,
        changes: false,
        menu: false,
        prompt: false,
        find: None,
        diff_split: false,
        file: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut val = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--version" => {
                println!("ffwd {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--snapshot" => a.snapshot = Some(PathBuf::from(val("--snapshot")?)),
            // Writes the app icon as a PNG (for packaging), then exits.
            "--icon-png" => {
                let path = PathBuf::from(val("--icon-png")?);
                let n: usize = it.next().and_then(|s| s.parse().ok()).unwrap_or(256);
                let rgba = icon::mark_rgba(n);
                snapshot::write_rgba_png(&path, &rgba, n, n)?;
                std::process::exit(0);
            }
            "--smoke" => a.smoke = true,
            "--size" => {
                let v = val("--size")?;
                let (w, h) = v.split_once('x').ok_or("--size is WxH")?;
                a.size = (w.parse().map_err(|_| "bad width")?, h.parse().map_err(|_| "bad height")?);
            }
            "--scale" => a.scale = Some(val("--scale")?.parse().map_err(|_| "bad scale")?),
            "--select" => a.select = Some(val("--select")?.parse().map_err(|_| "bad row")?),
            "--focus" => a.focus = Some(val("--focus")?),
            "--palette" => a.palette = Some(val("--palette")?),
            "--layout" => a.layout = Some(val("--layout")?),
            "--settings" => a.settings = true,
            "--settings-page" => a.settings_page = Some(val("--settings-page")?.parse().map_err(|_| "bad page")?),
            "--agents-demo" => a.agents_demo = true,
            "--settings-scope-demo" => a.scope_demo = true,
            "--changes" => a.changes = true,
            "--menu" => a.menu = true,
            "--prompt" => a.prompt = true,
            "--find" => a.find = Some(val("--find")?),
            "--diff-split" => a.diff_split = true,
            "--file" => a.file = Some(val("--file")?.parse().map_err(|_| "bad file index")?),
            "--hover" => {
                let v = val("--hover")?;
                let (x, y) = v.split_once(',').ok_or("--hover is X,Y")?;
                a.hover = Some((x.parse().map_err(|_| "bad x")?, y.parse().map_err(|_| "bad y")?));
            }
            "--side" => a.side = Some(val("--side")?),
            "--picker" => a.picker = Some(val("--picker")?),
            "--tab" => a.tabs.push(PathBuf::from(val("--tab")?)),
            "-h" | "--help" => {
                println!(
                    "usage: ffwd [repo] [--snapshot out.png [--size WxH] [--scale S] [--select N] [--focus files|diff] [--palette Q]] [--smoke]"
                );
                std::process::exit(0);
            }
            s if s.starts_with('-') => return Err(format!("unknown option {s}")),
            s => a.repo = Some(PathBuf::from(s)),
        }
    }
    Ok(a)
}

static TRACE: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
static TRACE_ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// Records a startup or frame mark. Marks are buffered and printed at exit:
/// writing each one to a redirected stderr cost ~17 ms per line on Windows
/// and distorted the very timings it reported.
pub fn trace(t0: Instant, what: &str) {
    if *TRACE_ON.get_or_init(|| std::env::var_os("FFWD_TRACE").is_some()) {
        let line = format!("{:>8.2} ms  {what}", t0.elapsed().as_secs_f64() * 1000.0);
        if let Ok(mut v) = TRACE.lock() {
            v.push(line);
        }
    }
}

fn flush_trace() {
    if let Ok(v) = TRACE.lock() {
        for l in v.iter() {
            eprintln!("{l}");
        }
    }
}

fn main() {
    let t0 = Instant::now();
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("ffwd: {e}");
            std::process::exit(2);
        }
    };
    let code = if args.snapshot.is_some() { snapshot::run(&args, t0) } else { window::run(&args, t0) };
    // Agents ffwd started end with it (only the PIDs it recorded).
    ffwd_agent::proc::kill_all();
    flush_trace();
    std::process::exit(code);
}
