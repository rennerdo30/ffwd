//! Synthetic in-memory demo repository and component reference sheet for
//! sub-second UI iteration (`--demo`, `--snapshot-ui`, `--serve`).
//!
//! Feeds real `Msg` values through `Ui::on_tab_msg` with zero `git.exe` spawns
//! and zero disk I/O, so every screen renders with the exact production paint
//! code in a few milliseconds and stays free of local paths or personal data.

use crate::theme;
use crate::ui::{ActivityEntry, FileLoaded, Glyph, Ui};
use crate::work::Msg;
use ffwd_core::changes::{Entry as ChangeEntry, WorkStatus};
use ffwd_core::conflict;
use ffwd_core::diff::{Details, DiffLine, FileChange, FileDiff, LineKind};
use ffwd_core::files::{Content, Tree};
use ffwd_core::git::Ran;
use ffwd_core::hosting::{Ci, PullRequest};
use ffwd_core::rebase::{Action as RebaseAction, Step as RebaseStep};
use ffwd_core::reflog::Entry as ReflogEntry;
use ffwd_core::refs::{Ref, RefKind, Refs, StatusSummary};
use ffwd_core::remotes::Remote;
use ffwd_core::{Commit, Oid};

/// Every built-in screen rendered by `--snapshot-ui`.
pub const SCREENS: &[&str] = &[
    "history",
    "changes",
    "diff-split",
    "conflict",
    "files",
    "settings",
    "agents",
    "palette",
    "activity",
    "reflog",
    "rebase",
    "sheet",
];

fn oid(n: u8) -> Oid {
    let hex = format!("{n:02x}{:02x}a7c3d9e104b8265f0192837465a1b2c3d4e5", n.wrapping_mul(37).wrapping_add(11));
    Oid::from_hex(hex.as_bytes()).unwrap_or_default()
}

/// Populates `ui` (tab 1) with a rich synthetic repository in memory.
pub fn populate(ui: &mut Ui) {
    let now = 1_795_780_000i64;
    let c = |n: u8, parents: &[u8], author: &str, mins_ago: i64, subject: &str, has_body: bool| Commit {
        id: oid(n),
        parents: parents.iter().map(|&p| oid(p)).collect(),
        author: author.into(),
        email: format!("{}@example.dev", author.to_ascii_lowercase().replace(' ', ".")).into(),
        time: now - mins_ago * 60,
        subject: subject.into(),
        has_body,
    };

    // Multi-branch graph with a merge, feature branch, fix branch, tags, and stash.
    let commits = vec![
        c(1, &[2], "Alex Rivera", 8, "feat(ui): tighten toolbar rhythm and chip contrast", true),
        c(2, &[3, 6], "Alex Rivera", 32, "Merge branch 'feature/diff-overview' into main", false),
        c(3, &[4], "Maya Lin", 75, "fix(graph): match column width to visible lanes", true),
        c(4, &[5], "Samir Patel", 140, "perf(render): skip off-clip glyph rasterisation", false),
        c(5, &[8], "Alex Rivera", 260, "feat(changes): stage individual lines across hunks", true),
        c(6, &[7], "Maya Lin", 95, "feat(diff): clickable overview strip with change ticks", true),
        c(7, &[8], "Maya Lin", 190, "refactor(diff): share syntax state across unified and split", false),
        c(8, &[9], "Samir Patel", 420, "Release 0.2.0", false),
        c(9, &[10, 12], "Alex Rivera", 600, "Merge branch 'fix/scroll-damage' into main", false),
        c(10, &[11], "Maya Lin", 720, "feat(sidebar): fold branch folders and remember state", false),
        c(11, &[13], "Samir Patel", 960, "fix(win): keep Snap Layouts active on custom caption", true),
        c(12, &[13], "Alex Rivera", 680, "fix(scroll): clamp BitBlt band inside viewport bounds", false),
        c(13, &[14], "Alex Rivera", 1500, "feat(search): stream content and path matches", false),
        c(14, &[15], "Maya Lin", 2400, "feat(reflog): highlight unreachable commits for recovery", false),
        c(15, &[16], "Samir Patel", 3600, "feat(rebase): interactive plan without opening an editor", true),
        c(16, &[17], "Alex Rivera", 5200, "Release 0.1.9", false),
        c(17, &[18], "Maya Lin", 7200, "feat(theme): warm paper light theme matching contrast steps", false),
        c(18, &[], "Alex Rivera", 9800, "Initial repository layout and damage-only canvas", false),
    ];

    let rref =
        |kind: RefKind, name: &str, target: u8, head: bool, up: Option<&str>, ahead: u32, behind: u32, mins: i64| Ref {
            kind,
            name: name.into(),
            target: oid(target),
            upstream: up.map(Into::into),
            upstream_gone: false,
            ahead,
            behind,
            is_head: head,
            time: now - mins * 60,
        };

    let refs = Refs {
        head_branch: Some("main".into()),
        head: Some(oid(1)),
        refs: vec![
            rref(RefKind::Local, "main", 1, true, Some("origin/main"), 2, 0, 8),
            rref(RefKind::Local, "feature/ui-kit", 6, false, Some("origin/feature/ui-kit"), 1, 0, 95),
            rref(RefKind::Local, "fix/scroll-damage", 12, false, None, 0, 0, 680),
            rref(RefKind::Local, "release/0.2", 8, false, Some("origin/release/0.2"), 0, 0, 420),
            rref(RefKind::Remote, "origin/main", 3, false, None, 0, 0, 75),
            rref(RefKind::Remote, "origin/feature/ui-kit", 7, false, None, 0, 0, 190),
            rref(RefKind::Remote, "origin/release/0.2", 8, false, None, 0, 0, 420),
            rref(RefKind::Tag, "v0.2.0", 8, false, None, 0, 0, 420),
            rref(RefKind::Tag, "v0.1.9", 16, false, None, 0, 0, 5200),
            rref(RefKind::Stash, "stash@{0}", 3, false, None, 0, 0, 50),
        ],
        stashes: vec![("stash@{0}".into(), "WIP on main: sidebar density tokens".into())],
        worktrees: vec!["/demo/ffwd".into(), "/demo/ffwd-wt/ui-kit".into()],
        worktree_branches: vec!["main".into(), "feature/ui-kit".into()],
        remotes: vec![Remote {
            name: "origin".into(),
            fetch_url: "https://github.com/example/ffwd.git".into(),
            push_url: None,
        }],
        submodules: Vec::new(),
        key: Some(42),
        common_dir: None,
    };

    ui.r.repo_name = "ffwd".into();
    if let Some(t) = ui.tabs.first_mut() {
        t.name = "ffwd".into();
    }
    ui.on_tab_msg(1, Msg::Log { batch: commits, done: true, elapsed_ms: 4.2 });
    ui.on_tab_msg(1, Msg::Refs(Ok(refs)));
    ui.on_tab_msg(1, Msg::Status(StatusSummary { staged: 2, unstaged: 3, untracked: 1, conflicted: 0 }));
    ui.on_tab_msg(1, Msg::SyncMarks(vec![oid(1), oid(2)], vec![]));
    ui.on_tab_msg(
        1,
        Msg::CiRuns(vec![
            (oid(1).to_string(), Ci::Running, "https://example.invalid/runs/101".into()),
            (oid(2).to_string(), Ci::Passed, "https://example.invalid/runs/100".into()),
            (oid(3).to_string(), Ci::Passed, "https://example.invalid/runs/99".into()),
            (oid(6).to_string(), Ci::Failed, "https://example.invalid/runs/98".into()),
        ]),
    );
    ui.on_tab_msg(
        1,
        Msg::PullRequests(Ok(vec![
            PullRequest {
                number: 42,
                title: "UI component kit and reference sheet".into(),
                head: "feature/ui-kit".into(),
                author: "maya".into(),
                draft: false,
                url: "https://example.invalid/pr/42".into(),
            },
            PullRequest {
                number: 43,
                title: "Sub-second token hot reload".into(),
                head: "feat/tokens".into(),
                author: "alex".into(),
                draft: true,
                url: "https://example.invalid/pr/43".into(),
            },
        ])),
    );

    // Populate commit details + diff for the selected commit (row 0).
    let details = Details {
        id: oid(1),
        parents: vec![oid(2)],
        author: "Alex Rivera".into(),
        email: "alex.rivera@example.dev".into(),
        time: now - 8 * 60,
        committer: "Alex Rivera".into(),
        commit_time: now - 8 * 60,
        message: "feat(ui): tighten toolbar rhythm and chip contrast\n\nBalances surface contrast steps between the sidebar, commit list, and\ndiff header while keeping every fill solid for cheap remote encoding.\n\n- Align ref chips on a consistent 20 px height\n- Boost muted metadata to WCAG AA 5:1 on both themes".into(),
        files: vec![
            FileChange { status: b'M', path: "crates/ffwd-app/src/theme.rs".into(), old_path: None, added: Some(48), removed: Some(19) },
            FileChange { status: b'M', path: "crates/ffwd-app/src/ui.rs".into(), old_path: None, added: Some(112), removed: Some(64) },
            FileChange { status: b'M', path: "crates/ffwd-app/src/ui_sidebar.rs".into(), old_path: None, added: Some(34), removed: Some(12) },
            FileChange { status: b'A', path: "crates/ffwd-app/src/demo.rs".into(), old_path: None, added: Some(210), removed: Some(0) },
            FileChange { status: b'R', path: "crates/ffwd-app/src/ui_diff.rs".into(), old_path: Some("crates/ffwd-app/src/diff_pane.rs".into()), added: Some(8), removed: Some(4) },
        ],
        files_truncated: false,
    };
    let diff = sample_diff("crates/ffwd-app/src/theme.rs");
    ui.r.details = Some(Box::new(details));
    ui.r.diff = Some(Box::new(diff));
    ui.r.sel = 0;
    ui.r.sel_file = 0;

    // Seed activity log entries (one pass, one failure).
    ui.on_tab_msg(
        1,
        Msg::Activity(Box::new(ActivityEntry {
            at: now - 120,
            repo: "ffwd".into(),
            title: "Fetch origin".into(),
            ok: true,
            summary: "2 refs updated".into(),
            ms: 410,
            ran: vec![Ran {
                args: "fetch --prune origin".into(),
                code: Some(0),
                stderr: "From https://github.com/example/ffwd\n   f2a7c3d..012da7c  main -> origin/main\n".into(),
                ms: 405,
            }],
        })),
    );
    ui.on_tab_msg(
        1,
        Msg::Activity(Box::new(ActivityEntry {
            at: now - 45,
            repo: "ffwd".into(),
            title: "Push feature/ui-kit".into(),
            ok: false,
            summary: "non-fast-forward rejected".into(),
            ms: 620,
            ran: vec![Ran {
                args: "push origin HEAD:refs/heads/feature/ui-kit".into(),
                code: Some(1),
                stderr:
                    "! [rejected] feature/ui-kit -> feature/ui-kit (fetch first)\nerror: failed to push some refs\n"
                        .into(),
                ms: 615,
            }],
        })),
    );

    ui.resize(ui.w, ui.h, ui.s);
}

fn sample_diff(path: &str) -> FileDiff {
    let dl = |kind: LineKind, old: u32, new: u32, text: &str| DiffLine { kind, old, new, text: text.into() };
    FileDiff {
        path: path.into(),
        binary: false,
        truncated: false,
        lines: vec![
            dl(LineKind::Hunk, 0, 0, "@@ -42,14 +42,18 @@ pub struct Metrics {"),
            dl(LineKind::Context, 42, 42, " /// Layout and typography metrics that can be tuned live."),
            dl(LineKind::Context, 43, 43, " #[derive(Clone, Copy, Debug, PartialEq)]"),
            dl(LineKind::Context, 44, 44, " pub struct Metrics {"),
            dl(LineKind::Del, 45, 0, "     pub type_scale: f32,"),
            dl(LineKind::Del, 46, 0, "     pub row_h: f32,"),
            dl(LineKind::Add, 0, 45, "     /// Scales every font size across chrome and code panes."),
            dl(LineKind::Add, 0, 46, "     pub type_scale: f32,"),
            dl(LineKind::Add, 0, 47, "     /// History commit row height in design pixels."),
            dl(LineKind::Add, 0, 48, "     pub row_h: f32,"),
            dl(LineKind::Add, 0, 49, "     /// Sidebar item height in design pixels."),
            dl(LineKind::Add, 0, 50, "     pub side_row_h: f32,"),
            dl(LineKind::Context, 47, 51, "     pub top_bar_h: f32,"),
            dl(LineKind::Context, 48, 52, "     pub tab_bar_h: f32,"),
            dl(LineKind::Context, 49, 53, " }"),
            dl(LineKind::Hunk, 0, 0, "@@ -96,10 +100,12 @@ define_palette! {"),
            dl(LineKind::Context, 96, 100, "     TEXT(text) = 0xeceae6, 0x1e1c19;"),
            dl(LineKind::Context, 97, 101, "     TEXT_2(text_2) = 0xc0beb8, 0x3b3833;"),
            dl(LineKind::Del, 98, 0, "     TEXT_3(text_3) = 0x96948e, 0x5f5b54;"),
            dl(LineKind::Del, 99, 0, "     TEXT_4(text_4) = 0x706e69, 0x7f7a71;"),
            dl(LineKind::Add, 0, 102, "     TEXT_3(text_3) = 0xa8a6a0, 0x58544d;"),
            dl(LineKind::Add, 0, 103, "     TEXT_4(text_4) = 0x8a8882, 0x726d64;"),
            dl(LineKind::Context, 100, 104, "     /// Primary brand accent (active tab, focus, HEAD)."),
            dl(LineKind::Context, 101, 105, "     ACCENT(accent) = 0xe8a33e, 0xd98d22;"),
        ],
    }
}

fn populate_changes(ui: &mut Ui, with_conflict: bool) {
    let mut entries = vec![
        ChangeEntry {
            path: "crates/ffwd-app/src/theme.rs".into(),
            old_path: None,
            staged: b'.',
            unstaged: b'M',
            conflicted: false,
        },
        ChangeEntry {
            path: "crates/ffwd-app/src/ui_sidebar.rs".into(),
            old_path: None,
            staged: b'.',
            unstaged: b'M',
            conflicted: false,
        },
        ChangeEntry {
            path: "docs/tokens.example.toml".into(),
            old_path: None,
            staged: b'.',
            unstaged: b'?',
            conflicted: false,
        },
        ChangeEntry {
            path: "crates/ffwd-app/src/demo.rs".into(),
            old_path: None,
            staged: b'A',
            unstaged: b'.',
            conflicted: false,
        },
        ChangeEntry { path: "Cargo.toml".into(), old_path: None, staged: b'M', unstaged: b'.', conflicted: false },
    ];
    if with_conflict {
        entries.insert(
            0,
            ChangeEntry {
                path: "crates/ffwd-app/src/ui.rs".into(),
                old_path: None,
                staged: b'U',
                unstaged: b'U',
                conflicted: true,
            },
        );
    }
    ui.r.changes_view = true;
    ui.on_tab_msg(1, Msg::Changes(Ok(WorkStatus { entries, truncated: false })));
    ui.r.changes.diff = Some(Box::new(sample_diff("crates/ffwd-app/src/theme.rs")));
    if with_conflict {
        let sample = "\
fn paint_toolbar(&mut self, cv: &mut Canvas) {
    let bar = self.lay.top;
    cv.fill(bar, BG_BAR());
<<<<<<< HEAD
    let gap = self.px(8.0);
    let pad = self.px(12.0);
=======
    let gap = self.px(6.0);
    let pad = self.px(10.0);
>>>>>>> feature/ui-kit
    self.paint_actions(cv, bar, gap, pad);
}

fn chip_radius(&self) -> f32 {
<<<<<<< HEAD
    self.pf(4.0)
=======
    self.pf(theme::metrics().radius_sm)
>>>>>>> feature/ui-kit
}
";
        let (segs, eol) = conflict::parse(sample);
        ui.demo_open_conflict("Merge", "crates/ffwd-app/src/ui.rs", segs, eol);
    }
}

fn populate_files(ui: &mut Ui) {
    ui.r.files_view = true;
    let paths = [
        ("Cargo.toml", b'M'),
        ("README.md", 0),
        ("crates/ffwd-app/src/demo.rs", b'A'),
        ("crates/ffwd-app/src/main.rs", 0),
        ("crates/ffwd-app/src/theme.rs", b'M'),
        ("crates/ffwd-app/src/ui.rs", 0),
        ("crates/ffwd-app/src/ui_sidebar.rs", b'M'),
        ("crates/ffwd-core/src/lib.rs", 0),
        ("crates/ffwd-render/src/lib.rs", 0),
        ("docs/UI-DESIGN.md", 0),
    ];
    let tree = Tree::build(paths.iter().map(|&(p, m)| (p.to_string(), m)));
    let lower: Vec<(u32, Box<str>)> =
        paths.iter().enumerate().map(|(i, &(p, _))| (i as u32 + 1, p.to_ascii_lowercase().into())).collect();
    ui.on_tab_msg(1, Msg::FilesTree(Ok((Box::new(tree), lower))));
    let lines: Vec<Box<str>> = vec![
        "//! CPU canvas for the ffwd UI: solid fills and anti-aliased vectors.".into(),
        "".into(),
        "use crate::Rect;".into(),
        "".into(),
        "pub fn blend(dst: u32, src: u32, a: u32) -> u32 {".into(),
        "    if a >= 255 {".into(),
        "        return src;".into(),
        "    }".into(),
        "    if a == 0 {".into(),
        "        return dst;".into(),
        "    }".into(),
        "    let inv = 255 - a;".into(),
        "    let rb = ((dst & 0xff00ff) * inv + (src & 0xff00ff) * a + 0x800080) >> 8 & 0xff00ff;".into(),
        "    let g = ((dst & 0x00ff00) * inv + (src & 0x00ff00) * a + 0x008000) >> 8 & 0x00ff00;".into(),
        "    rb | g".into(),
        "}".into(),
    ];
    let blocks = vec![false; lines.len()];
    ui.r.files.file = Some((1, "crates/ffwd-render/src/lib.rs".into()));
    ui.r.files.content =
        Some(FileLoaded { content: Content { lines, binary: false, truncated: false, bytes: 420 }, blocks });
}

fn populate_reflog(ui: &mut Ui) {
    let now = 1_795_780_000i64;
    ui.r.reflog_view = true;
    ui.on_tab_msg(
        1,
        Msg::Reflog(Ok(vec![
            ReflogEntry {
                id: oid(1),
                selector: "HEAD@{0}".into(),
                message: "commit: feat(ui): tighten toolbar rhythm".into(),
                time: now - 480,
            },
            ReflogEntry {
                id: oid(2),
                selector: "HEAD@{1}".into(),
                message: "merge feature/diff-overview: Merge made by ort".into(),
                time: now - 1920,
            },
            ReflogEntry {
                id: oid(99),
                selector: "HEAD@{2}".into(),
                message: "reset: moving to HEAD~1 (experiment dropped)".into(),
                time: now - 3600,
            },
            ReflogEntry {
                id: oid(3),
                selector: "HEAD@{3}".into(),
                message: "checkout: moving from feature/ui-kit to main".into(),
                time: now - 5400,
            },
            ReflogEntry {
                id: oid(6),
                selector: "HEAD@{4}".into(),
                message: "commit: feat(diff): clickable overview strip".into(),
                time: now - 6200,
            },
        ])),
    );
}

fn populate_rebase(ui: &mut Ui) {
    let steps = vec![
        RebaseStep {
            sha: oid(5).to_string(),
            subject: "feat(changes): stage individual lines across hunks".into(),
            action: RebaseAction::Pick,
        },
        RebaseStep {
            sha: oid(4).to_string(),
            subject: "perf(render): skip off-clip glyph rasterisation".into(),
            action: RebaseAction::Pick,
        },
        RebaseStep {
            sha: oid(3).to_string(),
            subject: "fix(graph): match column width to visible lanes".into(),
            action: RebaseAction::Pick,
        },
        RebaseStep {
            sha: oid(1).to_string(),
            subject: "feat(ui): tighten toolbar rhythm and chip contrast".into(),
            action: RebaseAction::Pick,
        },
    ];
    ui.demo_open_rebase(Some(oid(8).to_string()), steps);
}

/// Switches a demo-populated `Ui` to one of [`SCREENS`].
/// Returns `Ok(true)` when the screen is the component reference `"sheet"`,
/// `Ok(false)` for a standard `Ui` screen.
pub fn apply_screen(ui: &mut Ui, screen: &str) -> Result<bool, String> {
    ui.reset_views();
    match screen {
        "history" => Ok(false),
        "diff-split" => {
            ui.diff_split = true;
            Ok(false)
        }
        "changes" => {
            populate_changes(ui, false);
            Ok(false)
        }
        "conflict" => {
            populate_changes(ui, true);
            Ok(false)
        }
        "files" => {
            populate_files(ui);
            Ok(false)
        }
        "settings" => {
            ui.open_settings();
            Ok(false)
        }
        "agents" => {
            ui.snapshot_agents_demo();
            Ok(false)
        }
        "palette" => {
            ui.open_palette_with("feat");
            Ok(false)
        }
        "activity" => {
            ui.enter_activity();
            Ok(false)
        }
        "reflog" => {
            populate_reflog(ui);
            Ok(false)
        }
        "rebase" => {
            populate_rebase(ui);
            Ok(false)
        }
        "sheet" => Ok(true),
        other => Err(format!("unknown screen `{other}`; expected one of: {}", SCREENS.join(", "))),
    }
}

/// Paints a single-page design-token & component reference sheet (palette
/// surfaces, WCAG contrast ladder, semantic & lane colours, every vector glyph,
/// button states and ref chips) using the active theme and `Metrics`.
pub fn paint_sheet(ui: &mut Ui, cv: &mut ffwd_render::Canvas) {
    let b = cv.bounds();
    let p = theme::pal();
    let m = theme::metrics();
    let s = ui.s;
    let px = |v: f32| (v * s).round() as i32;
    let pf = |v: f32| v * s;
    let (fu, fs, fm) = (ui.f.ui, ui.f.semi, ui.f.mono);

    cv.fill(b, p.bg);
    let pad = px(28.0);
    let mut y = px(24.0);

    // Header
    let title = format!(
        "ffwd Design System Sheet  ·  {}  ·  type_scale {:.2}  row_h {:.0}  radius {:.0}/{:.0}/{:.0}",
        if theme::is_light() { "Light" } else { "Dark" },
        m.type_scale,
        m.row_h,
        m.radius_sm,
        m.radius_md,
        m.radius_lg
    );
    ui.te.draw(cv, fs, pf(16.0), pad as f32, (y + px(18.0)) as f32, &title, p.text, (b.w - 2 * pad) as f32);
    y += px(34.0);
    cv.fill(ffwd_render::Rect::new(pad, y, b.w - 2 * pad, 1), p.border);
    y += px(18.0);

    // 1. Surface swatches
    ui.te.draw(cv, fs, pf(12.0), pad as f32, (y + px(12.0)) as f32, "SURFACES & BORDERS", p.text_3, 400.0);
    y += px(22.0);
    let surfaces = [
        ("BG_SIDEBAR", p.bg_sidebar),
        ("BG", p.bg),
        ("BG_BAR", p.bg_bar),
        ("BG_PANEL", p.bg_panel),
        ("BG_INPUT", p.bg_input),
        ("BG_FLOAT", p.bg_float),
        ("BG_HOVER", p.bg_hover),
        ("BG_SELECT", p.bg_select),
        ("BORDER", p.border),
        ("BORDER_STR", p.border_strong),
        ("ACCENT", p.accent),
        ("ACCENT_SOFT", p.accent_soft),
    ];
    let sw_w = px(112.0);
    let sw_h = px(44.0);
    let sw_gap = px(8.0);
    let sw_cols = ((b.w - 2 * pad + sw_gap) / (sw_w + sw_gap)).max(1) as usize;
    let sw_rows = surfaces.len().div_ceil(sw_cols) as i32;
    for (i, &(name, col)) in surfaces.iter().enumerate() {
        let col_i = (i % sw_cols) as i32;
        let row_i = (i / sw_cols) as i32;
        let r = ffwd_render::Rect::new(pad + col_i * (sw_w + sw_gap), y + row_i * (sw_h + px(12.0)), sw_w, sw_h);
        cv.round_border(r, pf(m.radius_md), p.border, col);
        let fg = if theme::contrast_ratio(p.text, col) >= theme::contrast_ratio(p.accent_text, col) {
            p.text
        } else {
            p.accent_text
        };
        ui.te.draw(
            cv,
            fs,
            pf(10.0),
            (r.x + px(8.0)) as f32,
            (r.y + px(18.0)) as f32,
            name,
            fg,
            (r.w - px(12.0)) as f32,
        );
        let hex = format!("#{col:06x}");
        ui.te.draw(cv, fm, pf(9.5), (r.x + px(8.0)) as f32, (r.y + px(34.0)) as f32, &hex, fg, (r.w - px(12.0)) as f32);
    }
    y += sw_rows * (sw_h + px(12.0)) + px(12.0);

    // 2. Text contrast ladder (left) & Semantic/Diff + Graph lanes (right)
    let half_w = (b.w - 3 * pad) / 2;
    let right_x = pad + half_w + pad;
    ui.te.draw(
        cv,
        fs,
        pf(12.0),
        pad as f32,
        (y + px(12.0)) as f32,
        "TEXT CONTRAST LADDER (ON BG)",
        p.text_3,
        half_w as f32,
    );
    ui.te.draw(
        cv,
        fs,
        pf(12.0),
        right_x as f32,
        (y + px(12.0)) as f32,
        "SEMANTIC, DIFF & GRAPH LANES",
        p.text_3,
        half_w as f32,
    );
    let sec_y = y + px(22.0);
    let ladder = [
        ("TEXT (primary)", p.text),
        ("TEXT_2 (secondary)", p.text_2),
        ("TEXT_3 (metadata)", p.text_3),
        ("TEXT_4 (hints)", p.text_4),
        ("ACCENT_FG", p.accent_fg),
        ("ADD / DEL / MOD", p.add),
    ];
    for (i, &(label, col)) in ladder.iter().enumerate() {
        let ry = sec_y + i as i32 * px(26.0);
        let cr = theme::contrast_ratio(col, p.bg);
        let line = format!("{label}  ·  #{col:06x}  ·  {cr:.1}:1");
        ui.te.draw(cv, fu, pf(13.0), pad as f32, (ry + px(16.0)) as f32, &line, col, half_w as f32);
    }

    // Semantic chips & diff lines on the right
    let chips = [
        ("HEAD · main", p.accent_soft, p.accent, p.accent_fg, Glyph::Head),
        ("feature/ui-kit", p.bg_panel, p.border, p.local_fg, Glyph::Branch),
        ("origin/main", p.remote_bg, p.border, p.remote_fg, Glyph::Remote),
        ("v0.2.0", p.tag_bg, p.border, p.tag_fg, Glyph::Tag),
        ("stash@{0}", p.bg_panel, p.border, p.stash_fg, Glyph::Stash),
    ];
    let mut cx = right_x;
    for &(label, bg, bd, fg, g) in &chips {
        let tw = ui.te.measure(fs, pf(11.5), label).ceil() as i32;
        let r = ffwd_render::Rect::new(cx, sec_y, tw + px(28.0), px(24.0));
        cv.round_border(r, pf(m.radius_sm), bd, bg);
        ui.draw_glyph(cv, g, (r.x + px(12.0)) as f32, (r.y + r.h / 2) as f32, pf(5.5), fg);
        ui.te.draw(cv, fs, pf(11.5), (r.x + px(22.0)) as f32, (r.y + px(16.0)) as f32, label, fg, tw as f32 + 4.0);
        cx = r.right() + px(8.0);
    }
    // Diff preview strip
    let dy = sec_y + px(34.0);
    let diff_rows = [
        (p.hunk_bg, p.hunk_text, "@@ -42,6 +42,8 @@ fn paint_header()"),
        (p.diff_del_line, p.del_text, "-  let old_bg = 0x1c1d21;"),
        (p.diff_add_line, p.add_text, "+  let new_bg = theme::pal().bg;"),
    ];
    for (i, &(bg, fg, txt)) in diff_rows.iter().enumerate() {
        let r = ffwd_render::Rect::new(right_x, dy + i as i32 * px(22.0), half_w.min(px(560.0)), px(22.0));
        cv.fill(r, bg);
        ui.te.draw(
            cv,
            fm,
            pf(11.5),
            (r.x + px(10.0)) as f32,
            (r.y + px(15.0)) as f32,
            txt,
            fg,
            (r.w - px(16.0)) as f32,
        );
    }
    // Graph lanes
    let ly = dy + px(78.0);
    for i in 0..10u16 {
        let col = theme::lane(i);
        let lx = right_x as f32 + i as f32 * pf(32.0) + pf(12.0);
        cv.stroke(&[(lx, ly as f32), (lx, (ly + px(28.0)) as f32)], pf(2.5), col);
        cv.fill_circle(lx, (ly + px(14.0)) as f32, pf(5.0), col);
    }
    y = sec_y + px(168.0);

    // 3. Buttons matrix
    ui.te.draw(
        cv,
        fs,
        pf(12.0),
        pad as f32,
        (y + px(12.0)) as f32,
        "BUTTON STATES (REST · HOVER · PRESS)",
        p.text_3,
        500.0,
    );
    y += px(24.0);
    let btn_rows = [
        ("Primary", p.accent_soft, p.accent_border, p.accent_fg, p.accent_soft_hover, p.accent, p.accent_text),
        ("Secondary", p.bg_input, p.border, p.text, p.control_hover, p.bg_press, p.text),
        ("Ghost", p.bg, p.bg, p.text_2, p.bg_hover, p.bg_press, p.text),
        ("Danger", p.del_bg, p.danger_border, p.del_text, p.close_hover, p.close_press, p.del_text),
    ];
    for (row_i, &(name, r_bg, r_bd, r_fg, h_bg, p_bg, p_fg)) in btn_rows.iter().enumerate() {
        let bx = pad + row_i as i32 * px(300.0);
        let states =
            [("Rest", r_bg, r_bd, r_fg), ("Hover", h_bg, p.border_hover, r_fg), ("Press", p_bg, p.border_strong, p_fg)];
        for (j, &(st, bg, bd, fg)) in states.iter().enumerate() {
            let r = ffwd_render::Rect::new(bx + j as i32 * px(92.0), y, px(84.0), px(30.0));
            cv.round_border(r, pf(m.radius_md), bd, bg);
            let lbl = if j == 0 { name } else { st };
            ui.te.draw_in(cv, fs, pf(11.5), r, pf(8.0), lbl, fg);
        }
    }
    y += px(50.0);

    // 4. Every Glyph icon
    ui.te.draw(cv, fs, pf(12.0), pad as f32, (y + px(12.0)) as f32, "VECTOR ICONS (ui_icons.rs)", p.text_3, 400.0);
    y += px(24.0);
    let cw = px(114.0);
    let ch = px(40.0);
    let cgap = px(6.0);
    let cols = ((b.w - 2 * pad + cgap) / (cw + cgap)).max(1);
    for (i, &(name, g)) in Glyph::ALL.iter().enumerate() {
        let col_i = i as i32 % cols;
        let row_i = i as i32 / cols;
        let r = ffwd_render::Rect::new(pad + col_i * (cw + cgap), y + row_i * (ch + cgap), cw, ch);
        if r.bottom() > b.bottom() - px(10.0) {
            break;
        }
        cv.round_border(r, pf(m.radius_sm), p.border_soft, p.bg_panel);
        ui.draw_glyph(cv, g, (r.x + px(18.0)) as f32, (r.y + r.h / 2) as f32, pf(7.0), p.text_2);
        ui.te.draw(
            cv,
            fu,
            pf(10.5),
            (r.x + px(34.0)) as f32,
            (r.y + px(24.0)) as f32,
            name,
            p.text_3,
            (r.w - px(38.0)) as f32,
        );
    }
}
