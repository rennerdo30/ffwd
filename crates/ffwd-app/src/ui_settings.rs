//! The settings view (gear button, Ctrl+,). Every change is saved to
//! `settings.toml` at once, on a worker; the file stays hand-editable and
//! keeps its comments.

use super::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Row {
    Layout,
    Updates,
    Channel,
    Roots,
    AiEndpoint,
    AiModel,
    OpenFile,
    CheckUpdates,
    // Agents page (experimental).
    Scope,
    AgentsOn,
    Preset,
    AgentCommand,
    Sandbox,
    AllowUnsandboxed,
    Network,
    GitAccess,
    HomeAccess,
    ReadPaths,
    WritePaths,
    Setup,
    After,
    OnDone,
    Timeout,
    AgentsFile,
}

const GENERAL: &[Row] = &[
    Row::Layout,
    Row::Updates,
    Row::Channel,
    Row::Roots,
    Row::AiEndpoint,
    Row::AiModel,
    Row::OpenFile,
    Row::CheckUpdates,
];

const AGENT_ROWS: &[Row] = &[
    Row::Scope,
    Row::AgentsOn,
    Row::Preset,
    Row::AgentCommand,
    Row::Sandbox,
    Row::AllowUnsandboxed,
    Row::Network,
    Row::GitAccess,
    Row::HomeAccess,
    Row::ReadPaths,
    Row::WritePaths,
    Row::Setup,
    Row::After,
    Row::OnDone,
    Row::Timeout,
    Row::AgentsFile,
];

/// Settings pages: (title, rows).
const PAGES: [(&str, &[Row]); 2] = [("General", GENERAL), ("Agents", AGENT_ROWS)];

impl Row {
    fn label(self) -> &'static str {
        match self {
            Row::Layout => "Commit details",
            Row::Updates => "Automatic updates",
            Row::Channel => "Update channel",
            Row::Roots => "Repository folders",
            Row::AiEndpoint => "AI endpoint",
            Row::AiModel => "AI model",
            Row::OpenFile => "Settings file",
            Row::CheckUpdates => "Updates",
            Row::Scope => "Applies to",
            Row::AgentsOn => "Agents",
            Row::Preset => "Agent",
            Row::AgentCommand => "Agent command",
            Row::Sandbox => "Sandbox",
            Row::AllowUnsandboxed => "Allow no sandbox",
            Row::Network => "Network",
            Row::GitAccess => "Git access",
            Row::HomeAccess => "Home folder",
            Row::ReadPaths => "Extra folders, read-only",
            Row::WritePaths => "Extra folders, writable",
            Row::Setup => "Setup script",
            Row::After => "After script",
            Row::OnDone => "On-done script",
            Row::Timeout => "Time limit (minutes)",
            Row::AgentsFile => "All agent settings",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Row::Layout => "Where the selected commit's details and diff go",
            Row::Updates => "Installed copies check once a day",
            Row::Channel => "Automatic follows the running version",
            Row::Roots => "Folders searched for repositories in Open repository (; between folders)",
            Row::AiEndpoint => "OpenAI-compatible URL; local servers first (Ollama, LM Studio, llama.cpp)",
            Row::AiModel => "Model name the endpoint expects",
            Row::OpenFile => "Every setting, as text",
            Row::CheckUpdates => "",
            Row::Scope => "Every repository, or only this one (its overrides)",
            Row::AgentsOn => "Coding agents in their own worktree and sandbox",
            Row::Preset => "Built-in command and grants; Custom uses your command",
            Row::AgentCommand => "{prompt} {prompt_file} {worktree} {branch}, one argument each",
            Row::Sandbox => "Auto: bwrap (Linux), WSL (Windows), macOS sandbox",
            Row::AllowUnsandboxed => "Sandbox None: the agent has all your permissions",
            Row::Network => "Agents that call a hosted model need it",
            Row::GitAccess => "Read-only: ffwd commits the agent's edits at the end",
            Row::HomeAccess => "Private: an empty home; Read: for agent logins",
            Row::ReadPaths => "Applies from the next run (; between folders)",
            Row::WritePaths => "For example the agent's own config folder",
            Row::Setup => "Runs in the worktree before the agent, e.g. npm ci",
            Row::After => "Runs after the agent, before ffwd commits, e.g. tests",
            Row::OnDone => "On your machine when a run ends, e.g. a notification",
            Row::Timeout => "The agent is stopped after this (1 to 1440)",
            Row::AgentsFile => "Env, folders, branch prefix, prompt template, shells",
        }
    }

    fn key(self) -> Option<&'static str> {
        match self {
            Row::Roots => Some("repos.roots"),
            Row::AiEndpoint => Some("ai.endpoint"),
            Row::AiModel => Some("ai.model"),
            Row::AgentCommand => Some("agents.command"),
            Row::ReadPaths => Some("agents.read_paths"),
            Row::WritePaths => Some("agents.write_paths"),
            Row::Setup => Some("agents.setup_script"),
            Row::After => Some("agents.after_script"),
            Row::OnDone => Some("agents.on_done_script"),
            Row::Timeout => Some("agents.timeout_minutes"),
            _ => None,
        }
    }

    /// The `agents.*` key a row shows (text and choice rows).
    fn agent_key(self) -> Option<&'static str> {
        Some(match self {
            Row::AgentsOn => "agents.enabled",
            Row::Preset => "agents.preset",
            Row::Sandbox => "agents.sandbox",
            Row::AllowUnsandboxed => "agents.allow_unsandboxed",
            Row::Network => "agents.network",
            Row::GitAccess => "agents.git_access",
            Row::HomeAccess => "agents.home_access",
            r => return r.key().filter(|k| k.starts_with("agents.")),
        })
    }

    fn placeholder(self) -> &'static str {
        match self {
            Row::Roots => "Folders next to the open repositories",
            Row::AiEndpoint => "Off · e.g. http://localhost:11434/v1",
            Row::AiModel => "e.g. qwen2.5-coder:14b",
            Row::AgentCommand => "e.g. aider --yes-always --message-file {prompt_file}",
            Row::ReadPaths | Row::WritePaths => "None",
            Row::Setup | Row::After | Row::OnDone => "None",
            Row::Timeout => "60",
            _ => "",
        }
    }

    fn choices(self) -> &'static [&'static str] {
        match self {
            Row::Layout => &["Below the history", "Beside the history"],
            Row::Updates => &["On", "Off"],
            Row::Channel => &["Automatic", "Stable", "Beta"],
            Row::Scope => &["All repositories", "This repository"],
            Row::AgentsOn => &["Off", "On (experimental)"],
            Row::Preset => &["Custom", "Claude", "Codex", "agy", "opencode", "Muse"],
            Row::Sandbox => &["Auto", "bwrap", "WSL", "macOS", "Custom", "None"],
            Row::AllowUnsandboxed => &["No", "Yes"],
            Row::Network => &["On", "Off"],
            Row::GitAccess => &["Read-only", "Writable"],
            Row::HomeAccess => &["Private", "Read", "Write"],
            _ => &[],
        }
    }
}

pub struct SettingsView {
    page: usize,
    /// First row shown (the page scrolls when it does not fit).
    scroll: usize,
    sel: usize,
    /// The text row being edited and its field.
    edit: Option<(usize, TextField)>,
    /// Option hit areas from the last paint: (row, choice, rect).
    hits: Vec<(usize, usize, Rect)>,
    /// Page tabs from the last paint.
    page_hits: Vec<(usize, Rect)>,
}

const PRESET_IDS: [&str; 6] = ["custom", "claude", "codex", "antigravity", "opencode", "muse"];

const PAGE_AGENTS: usize = 1;

impl Ui {
    pub fn open_settings(&mut self) {
        // From the agents panel, Settings opens on the agents page.
        let page = if self.agent_view.take().is_some() { PAGE_AGENTS } else { 0 };
        self.open_settings_page(page);
    }

    pub fn open_settings_page(&mut self, page: usize) {
        self.palette = None;
        self.damage.full = true;
        self.settings_view = Some(SettingsView {
            page: page.min(PAGES.len() - 1),
            scroll: 0,
            sel: 0,
            edit: None,
            hits: Vec::new(),
            page_hits: Vec::new(),
        });
        self.agent_scope_repo = false;
        let r = self.settings_rect();
        self.damage.rects.push(r);
    }

    pub(super) fn close_settings(&mut self) {
        if self.settings_view.take().is_some() {
            let r = self.settings_rect();
            self.damage.rects.push(r);
        }
    }

    /// The settings element under the pointer (for hover).
    pub(super) fn settings_hit_at(&self, x: i32, y: i32) -> Option<Hot> {
        let v = self.settings_view.as_ref()?;
        if let Some(&(p, _)) = v.page_hits.iter().find(|h| h.1.contains(x, y)) {
            return Some(Hot::SettingsPage(p));
        }
        v.hits.iter().find(|h| h.2.contains(x, y)).map(|h| Hot::Setting(h.0, h.1))
    }

    pub(super) fn setting_is_text(&self, row: usize) -> bool {
        self.rows().get(row).is_some_and(|r| r.key().is_some())
    }

    fn rows(&self) -> &'static [Row] {
        PAGES[self.settings_view.as_ref().map_or(0, |v| v.page)].1
    }

    /// The tallest page sets the height, so switching pages keeps the box.
    pub(super) fn settings_rect(&self) -> Rect {
        let w = self.px(800.0).min(self.w as i32 - self.px(60.0));
        let most = PAGES.iter().map(|p| p.1.len()).max().unwrap_or(1) as i32;
        let h = (self.px(108.0) + most * self.px(54.0) + self.px(16.0)).min(self.h as i32 - self.px(60.0));
        Rect::new((self.w as i32 - w) / 2, (self.h as i32 - h) / 2, w, h)
    }

    fn settings_rows_visible(&self) -> usize {
        let r = self.settings_rect();
        ((r.h - self.px(108.0) - self.px(10.0)) / self.px(54.0)).max(1) as usize
    }

    /// Keeps the selected row on screen.
    fn settings_follow(&mut self) {
        let fit = self.settings_rows_visible();
        if let Some(v) = &mut self.settings_view {
            if v.sel < v.scroll {
                v.scroll = v.sel;
            } else if v.sel >= v.scroll + fit {
                v.scroll = v.sel + 1 - fit;
            }
        }
    }

    fn settings_page(&mut self, page: usize) {
        self.commit_edit();
        if let Some(v) = &mut self.settings_view {
            v.page = page % PAGES.len();
            if v.page != PAGE_AGENTS {
                self.agent_scope_repo = false;
            }
            v.sel = 0;
            v.scroll = 0;
            v.edit = None;
        }
        let r = self.settings_rect();
        self.damage.rects.push(r);
    }

    /// Editing this repository's overrides (Agents page, "This repository").
    fn repo_scope(&self) -> bool {
        self.agent_scope_repo && !self.tabs.is_empty()
    }

    /// A value as the settings view shows it: agent keys in repository
    /// scope come from the repository's overrides, then the global value.
    fn sget(&self, key: &str) -> Option<&str> {
        if key.starts_with("agents.")
            && self.repo_scope()
            && let Some(v) = self.current_repo_settings().and_then(|r| r.get(key))
        {
            return Some(v);
        }
        self.settings.get(key)
    }

    /// Saves a value: in repository scope, agent keys become overrides of
    /// this repository; everything else is global.
    fn sset(&mut self, key: &str, value: &str) {
        if key.starts_with("agents.")
            && self.repo_scope()
            && let Some(r) = self.current_repo_settings_mut()
        {
            r.set_string(key, value);
            return;
        }
        self.settings.set_string(key, value);
    }

    /// Does this repository override the row's value?
    fn overridden(&self, row: Row) -> bool {
        row.agent_key().is_some_and(|k| self.current_repo_settings().is_some_and(|r| r.get(k).is_some()))
    }

    fn flag_now(&self, key: &str, default: bool) -> bool {
        match self.sget(key) {
            Some("true" | "on" | "yes" | "1") => true,
            Some("false" | "off" | "no" | "0") => false,
            _ => default,
        }
    }

    fn choice_now(&self, row: Row) -> usize {
        match row {
            Row::Layout => (self.layout == DetailsLayout::Beside) as usize,
            Row::Updates => (self.sget("update.check") == Some("false")) as usize,
            Row::Channel => match self.sget("update.channel") {
                Some("stable") => 1,
                Some("beta") => 2,
                _ => 0,
            },
            Row::Scope => self.repo_scope() as usize,
            Row::AgentsOn => self.flag_now("agents.enabled", false) as usize,
            Row::Preset => {
                let v = self.sget("agents.preset").unwrap_or("custom").to_ascii_lowercase();
                let v = if v == "agy" { "antigravity".to_string() } else { v };
                PRESET_IDS.iter().position(|p| *p == v).unwrap_or(0)
            }
            Row::Sandbox => {
                let k = self.sget("agents.sandbox").and_then(ffwd_agent::SandboxKind::parse);
                ["auto", "bwrap", "wsl", "macos", "custom", "none"]
                    .iter()
                    .position(|s| Some(*s) == k.map(|k| k.as_setting()))
                    .unwrap_or(0)
            }
            Row::AllowUnsandboxed => self.flag_now("agents.allow_unsandboxed", false) as usize,
            Row::Network => !self.flag_now("agents.network", true) as usize,
            Row::GitAccess => (self.sget("agents.git_access") == Some("write")) as usize,
            Row::HomeAccess => match self.sget("agents.home_access") {
                Some("read") => 1,
                Some("write") => 2,
                _ => 0,
            },
            _ => 0,
        }
    }

    fn set_choice(&mut self, row: Row, i: usize) {
        match row {
            Row::Layout => self.set_layout(if i == 1 { DetailsLayout::Beside } else { DetailsLayout::Below }),
            Row::Updates => self.sset("update.check", if i == 1 { "false" } else { "true" }),
            Row::Channel => self.sset("update.channel", ["auto", "stable", "beta"][i.min(2)]),
            Row::Scope => {
                let has_tab = !self.tabs.is_empty();
                self.agent_scope_repo = i == 1 && has_tab;
                if i == 1 && !has_tab {
                    self.set_notice("Open a repository first: its overrides belong to it.".into());
                }
            }
            Row::Preset => self.sset("agents.preset", PRESET_IDS[i.min(5)]),
            Row::AgentsOn => {
                self.sset("agents.enabled", if i == 1 { "true" } else { "false" });
                if i == 1 {
                    // Runs recorded earlier show up again.
                    self.agents_loaded.clear();
                    self.load_agent_runs();
                }
            }
            Row::Sandbox => self.sset("agents.sandbox", ["auto", "bwrap", "wsl", "macos", "custom", "none"][i.min(5)]),
            Row::AllowUnsandboxed => self.sset("agents.allow_unsandboxed", if i == 1 { "true" } else { "false" }),
            Row::Network => self.sset("agents.network", if i == 1 { "false" } else { "true" }),
            Row::GitAccess => self.sset("agents.git_access", if i == 1 { "write" } else { "read" }),
            Row::HomeAccess => self.sset("agents.home_access", ["private", "read", "write"][i.min(2)]),
            _ => {}
        }
        let r = self.settings_rect();
        self.damage.rects.push(r);
    }

    fn settings_act(&mut self, i: usize) {
        let Some(&row) = self.rows().get(i) else { return };
        match row {
            Row::OpenFile | Row::AgentsFile => open_settings_file(),
            Row::CheckUpdates => {
                self.close_settings();
                self.request_update_check();
            }
            r if r.key().is_some() => {
                let cur = self.sget(r.key().unwrap_or_default()).unwrap_or("").to_string();
                if let Some(v) = &mut self.settings_view {
                    v.edit = Some((i, TextField::new(&cur)));
                }
            }
            r => {
                let n = r.choices().len().max(1);
                let next = (self.choice_now(r) + 1) % n;
                self.set_choice(r, next);
            }
        }
        let r = self.settings_rect();
        self.damage.rects.push(r);
    }

    fn commit_edit(&mut self) {
        let rows = self.rows();
        let Some(v) = &mut self.settings_view else { return };
        if let Some((i, f)) = v.edit.take()
            && let Some(key) = rows.get(i).and_then(|r| r.key())
        {
            let text = f.text().trim().to_string();
            self.sset(key, &text);
            if key == "repos.roots" {
                self.scan_repos();
            }
        }
    }

    /// All keys go to the settings view while it is open.
    pub(super) fn settings_key(&mut self, k: Key) {
        let rows = self.rows();
        let Some(v) = &mut self.settings_view else { return };
        if let Some((_, f)) = &mut v.edit {
            match k {
                Key::Enter | Key::Tab => self.commit_edit(),
                Key::Esc => v.edit = None,
                Key::Char(c) => f.insert(c.encode_utf8(&mut [0; 4])),
                Key::Backspace => f.backspace(),
                Key::CtrlBackspace => f.delete_word(),
                Key::Delete => f.delete(),
                Key::Left => f.left(),
                Key::Right => f.right(),
                Key::Home => f.home(),
                Key::End => f.end(),
                Key::Ctrl('a') => f.select_all(),
                Key::Ctrl('c') => crate::platform::set_clipboard(f.copy_text()),
                Key::Ctrl('v') => {
                    if let Some(t) = crate::platform::clipboard() {
                        f.insert(&t);
                    }
                }
                _ => {}
            }
            let r = self.settings_rect();
            self.damage.rects.push(r);
            return;
        }
        let sel = v.sel.min(rows.len() - 1);
        match k {
            Key::Esc | Key::Ctrl(',') => self.close_settings(),
            Key::NextTab | Key::PrevTab => {
                let p = v.page;
                self.settings_page(p + 1);
            }
            Key::Up | Key::BackTab => v.sel = sel.saturating_sub(1),
            Key::Down | Key::Tab => v.sel = (sel + 1).min(rows.len() - 1),
            Key::Delete if self.agent_scope_repo && v.page == PAGE_AGENTS => {
                if let Some(k) = rows[sel].agent_key()
                    && let Some(r) = self.current_repo_settings_mut()
                {
                    r.remove(k);
                }
            }
            Key::PageUp | Key::Home => v.sel = 0,
            Key::PageDown | Key::End => v.sel = rows.len() - 1,
            Key::Enter | Key::Char(' ') => self.settings_act(sel),
            Key::Left | Key::Right if !rows[sel].choices().is_empty() => {
                let n = rows[sel].choices().len();
                let cur = self.choice_now(rows[sel]);
                let next = if matches!(k, Key::Left) { (cur + n - 1) % n } else { (cur + 1) % n };
                self.set_choice(rows[sel], next);
            }
            Key::Char(c) if rows[sel].key().is_some() => {
                // Typing on a text row starts editing it.
                self.settings_act(sel);
                if let Some(SettingsView { edit: Some((_, f)), .. }) = &mut self.settings_view {
                    f.select_all();
                    f.insert(c.encode_utf8(&mut [0; 4]));
                }
            }
            _ => {}
        }
        self.settings_follow();
        let r = self.settings_rect();
        self.damage.rects.push(r);
    }

    pub(super) fn settings_wheel(&mut self, lines: i64) {
        let n = self.rows().len();
        let fit = self.settings_rows_visible();
        if let Some(v) = &mut self.settings_view {
            v.scroll = (v.scroll as i64 + lines).clamp(0, n.saturating_sub(fit) as i64) as usize;
            v.sel = v.sel.clamp(v.scroll, (v.scroll + fit).min(n) - 1);
        }
        let r = self.settings_rect();
        self.damage.rects.push(r);
    }

    pub(super) fn settings_click(&mut self, x: i32, y: i32) {
        let r = self.settings_rect();
        if !r.contains(x, y) {
            self.commit_edit();
            self.close_settings();
            return;
        }
        let page =
            self.settings_view.as_ref().and_then(|v| v.page_hits.iter().find(|h| h.1.contains(x, y)).map(|h| h.0));
        if let Some(p) = page {
            return self.settings_page(p);
        }
        let rows = self.rows();
        let hit = self.settings_view.as_ref().and_then(|v| v.hits.iter().find(|h| h.2.contains(x, y)).copied());
        if let Some((row, choice, _)) = hit {
            self.commit_edit();
            if let Some(v) = &mut self.settings_view {
                v.sel = row;
            }
            if rows[row].choices().is_empty() {
                self.settings_act(row);
            } else {
                self.set_choice(rows[row], choice);
            }
        }
        self.damage.rects.push(r);
    }

    pub(super) fn paint_settings(&mut self, cv: &mut Canvas) {
        let Some(view) = self.settings_view.take() else { return };
        let r = self.settings_rect();
        let old = cv.set_clip(cv.clip().intersect(r));
        cv.round_border(r, self.pf(12.0), BORDER_STRONG, BG_PANEL);
        let (ui, semi, mono) = (self.f.ui, self.f.semi, self.f.mono);
        let pad = self.pf(28.0);
        let x0 = r.x as f32 + pad;
        let title_y = r.y as f32 + self.pf(40.0);
        let e = self.te.draw(cv, semi, self.pf(18.0), x0, title_y, "Settings", TEXT, self.pf(200.0));
        let ver = format!("ffwd {}", env!("CARGO_PKG_VERSION"));
        self.te.draw(cv, mono, self.pf(12.0), e + self.pf(14.0), title_y, &ver, TEXT_4, self.pf(200.0));
        let hint = "Esc closes · saved at once";
        let hw = self.te.measure(ui, self.pf(11.5), hint);
        self.te.draw(cv, ui, self.pf(11.5), r.right() as f32 - pad - hw, title_y, hint, TEXT_4, hw + 2.0);
        // Page tabs.
        let mut page_hits = Vec::new();
        let tab_y = r.y + self.px(58.0);
        let mut tx = x0;
        for (p, (name, _)) in PAGES.iter().enumerate() {
            let on = p == view.page;
            let w = self.te.measure(semi, self.pf(13.0), name);
            let extra = if p == PAGE_AGENTS { self.pf(98.0) } else { 0.0 };
            let tr = Rect::new(tx as i32 - self.px(10.0), tab_y, (w + extra) as i32 + self.px(20.0), self.px(32.0));
            if on {
                cv.fill_round(tr, self.pf(7.0), BG_CHOSEN);
            } else if self.is_hot(Hot::SettingsPage(p)) {
                cv.fill_round(tr, self.pf(7.0), BG_HOVER);
            }
            let base = (tab_y as f32 + self.pf(21.0)).round();
            let e = self.te.draw(cv, semi, self.pf(13.0), tx, base, name, if on { TEXT } else { TEXT_3 }, w + 2.0);
            if p == PAGE_AGENTS {
                let size = self.pf(9.5);
                let label = "EXPERIMENTAL";
                let pw = self.te.measure(semi, size, label) + self.pf(12.0);
                let pr = Rect::new((e + self.pf(8.0)) as i32, tab_y + self.px(8.0), pw as i32, self.px(16.0));
                cv.fill_round(pr, self.pf(8.0), TAG_BG);
                let pb = (pr.y as f32 + self.pf(11.5)).round();
                self.te.draw(cv, semi, size, pr.x as f32 + self.pf(6.0), pb, label, TAG_FG, pw);
            }
            page_hits.push((p, tr));
            tx = tr.right() as f32 + self.pf(14.0);
        }
        let hint2 = "Ctrl+Tab switches pages";
        let hw2 = self.te.measure(ui, self.pf(11.0), hint2);
        let hb = (tab_y as f32 + self.pf(21.0)).round();
        self.te.draw(cv, ui, self.pf(11.0), r.right() as f32 - pad - hw2, hb, hint2, TEXT_4, hw2 + 2.0);
        cv.fill(Rect::new(r.x + self.px(12.0), tab_y + self.px(40.0), r.w - self.px(24.0), 1), BORDER_SOFT);
        let rows = PAGES[view.page].1;
        let fit = self.settings_rows_visible();
        let mut hits = Vec::new();
        let rh = self.px(54.0);
        let top = r.y + self.px(108.0);
        let ctrl_x = r.x + (r.w as f32 * 0.44) as i32;
        let ctrl_w = r.right() - self.px(24.0) - ctrl_x;
        for (i, row) in rows.iter().enumerate().skip(view.scroll).take(fit) {
            let y = top + (i - view.scroll) as i32 * rh;
            let rr = Rect::new(r.x + self.px(12.0), y, r.w - self.px(24.0), rh - self.px(4.0));
            if i == view.sel {
                cv.fill_round(rr, self.pf(8.0), BG_SELECT_DIM);
            }
            let cy = y as f32 + rh as f32 / 2.0 - self.pf(2.0);
            // This repository overrides the row: accent in its scope, dim
            // in the global one (where the global value does not apply to it).
            let over = view.page == PAGE_AGENTS && self.overridden(*row);
            if over {
                let c = if self.repo_scope() { ACCENT } else { TEXT_4 };
                cv.fill_circle(x0 - self.pf(10.0), cy - self.pf(6.5), self.pf(3.0), c);
            }
            self.te.draw(
                cv,
                semi,
                self.pf(13.0),
                x0,
                cy - self.pf(2.0),
                row.label(),
                TEXT,
                (ctrl_x - r.x) as f32 - pad,
            );
            let repo_name = self.r.repo_name.clone();
            let hint = match *row {
                Row::Scope if self.repo_scope() => {
                    format!("Overrides for {repo_name} (marked); Delete resets a row")
                }
                Row::Scope => "Values for every repository without its own".to_string(),
                _ if over && !self.repo_scope() => format!("{repo_name} overrides this"),
                r => r.hint().to_string(),
            };
            self.te.draw(
                cv,
                ui,
                self.pf(11.5),
                x0,
                cy + self.pf(14.0),
                &hint,
                TEXT_3,
                (ctrl_x - r.x) as f32 - pad - self.pf(8.0),
            );
            let ctrl = Rect::new(ctrl_x, (cy - self.pf(15.0)) as i32, ctrl_w, self.px(30.0));
            let base = (cy + self.pf(4.5)).round();
            match *row {
                Row::OpenFile | Row::CheckUpdates | Row::AgentsFile => {
                    let label = if *row == Row::CheckUpdates { "Check for updates now" } else { "Open settings.toml" };
                    let w = self.te.measure(semi, self.pf(12.5), label) + self.pf(28.0);
                    let b = Rect::new(ctrl.x, ctrl.y, w as i32, ctrl.h);
                    let (bg, border) = self.control_colors(Hot::Setting(i, 0), BG_BAR, BORDER);
                    cv.round_border(b, self.pf(8.0), border, bg);
                    self.te.draw(cv, semi, self.pf(12.5), b.x as f32 + self.pf(14.0), base, label, TEXT, w);
                    hits.push((i, 0, b));
                }
                r2 if r2.key().is_some() => {
                    let editing = view.edit.as_ref().filter(|(j, _)| *j == i).map(|(_, f)| f.clone());
                    let border = if editing.is_some() {
                        ACCENT
                    } else if self.is_hot(Hot::Setting(i, 0)) {
                        BORDER_HOVER
                    } else {
                        BORDER
                    };
                    cv.round_border(ctrl, self.pf(7.0), border, BG_INPUT);
                    let tx = ctrl.x as f32 + self.pf(12.0);
                    let maxw = ctrl.w as f32 - self.pf(24.0);
                    match editing {
                        Some(f) => {
                            if f.all_selected() {
                                let w = self.te.measure(ui, self.pf(12.5), f.text()).min(maxw);
                                cv.fill_round(
                                    Rect::new(
                                        tx as i32 - 2,
                                        ctrl.y + self.px(6.0),
                                        w as i32 + 4,
                                        ctrl.h - self.px(12.0),
                                    ),
                                    self.pf(3.0),
                                    BG_TEXT_SELECT,
                                );
                            }
                            self.te.draw(cv, ui, self.pf(12.5), tx, base, f.text(), TEXT, maxw);
                            let cx = tx + self.te.measure(ui, self.pf(12.5), &f.text()[..f.cursor()]);
                            cv.fill(
                                Rect::new(
                                    cx as i32 + 1,
                                    ctrl.y + self.px(7.0),
                                    self.px(1.5).max(1),
                                    ctrl.h - self.px(14.0),
                                ),
                                ACCENT,
                            );
                        }
                        None => {
                            let v = self.sget(r2.key().unwrap_or_default()).unwrap_or("").to_string();
                            // The preset's command is what runs while this is empty.
                            let preset = (r2 == Row::AgentCommand)
                                .then(|| self.sget("agents.preset").and_then(ffwd_agent::presets::find))
                                .flatten();
                            let ph = match preset {
                                Some(p) => format!("{} preset: {}", p.name, p.command),
                                None => r2.placeholder().to_string(),
                            };
                            if v.is_empty() {
                                self.te.draw(cv, ui, self.pf(12.5), tx, base, &ph, TEXT_4, maxw);
                            } else {
                                self.te.draw(cv, ui, self.pf(12.5), tx, base, &v, TEXT, maxw);
                            }
                        }
                    }
                    hits.push((i, 0, ctrl));
                }
                r2 => {
                    // Segmented control.
                    let choices = r2.choices();
                    let now = self.choice_now(r2);
                    let seg_w = ctrl.w / choices.len().max(1) as i32;
                    cv.round_border(ctrl, self.pf(7.0), BORDER, BG_INPUT);
                    for (c, name) in choices.iter().enumerate() {
                        let s = Rect::new(ctrl.x + c as i32 * seg_w, ctrl.y, seg_w, ctrl.h);
                        if c == now {
                            cv.fill_round(s.inset(self.px(3.0), self.px(3.0)), self.pf(5.0), BG_CHOSEN);
                        } else if self.is_hot(Hot::Setting(i, c)) {
                            cv.fill_round(s.inset(self.px(3.0), self.px(3.0)), self.pf(5.0), CONTROL_HOVER);
                        }
                        let tw = self.te.measure(if c == now { semi } else { ui }, self.pf(12.5), name);
                        let col = if c == now { TEXT } else { TEXT_3 };
                        self.te.draw(
                            cv,
                            if c == now { semi } else { ui },
                            self.pf(12.5),
                            s.x as f32 + (s.w as f32 - tw) / 2.0,
                            base,
                            name,
                            col,
                            s.w as f32,
                        );
                        hits.push((i, c, s));
                    }
                }
            }
        }
        if rows.len() > fit {
            // A thin scroll indicator on the right edge.
            let track = Rect::new(r.right() - self.px(8.0), top, self.px(3.0), fit as i32 * rh);
            let th = (track.h as f32 * fit as f32 / rows.len() as f32) as i32;
            let ty2 = track.y + (track.h - th) * view.scroll as i32 / (rows.len() - fit).max(1) as i32;
            cv.fill_round(Rect::new(track.x, ty2, track.w, th), self.pf(1.5), TEXT_4);
        }
        cv.set_clip(old);
        self.settings_view = Some(SettingsView { hits, page_hits, ..view });
    }
}

/// Opens settings.toml in the default editor (creating it with a short,
/// commented template first). Runs on a worker: file and process work stay
/// off the UI thread.
fn open_settings_file() {
    let Some(path) = crate::settings::path() else { return };
    std::thread::spawn(move || {
        if !path.exists() {
            if let Some(d) = path.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = std::fs::write(
                &path,
                "# ffwd settings. ffwd keeps your comments when it changes a value.\n\n[ui]\n# layout = \"below\"   # or \"beside\"\n\n[update]\n# check = true\n# channel = \"auto\"   # \"stable\" or \"beta\"\n\n[repos]\n# roots = \"E:\\\\Development;D:\\\\Work\"\n\n[ai]\n# endpoint = \"http://localhost:11434/v1\"\n# model = \"qwen2.5-coder:14b\"\n\n[agents]\n# EXPERIMENTAL. Every key: docs/AGENTS-EXPERIMENTAL.md\n# enabled = false\n# command = \"aider --yes-always --message-file {prompt_file}\"\n# sandbox = \"auto\"   # bwrap, wsl, macos, custom, none\n",
            );
        }
        #[cfg(windows)]
        let _ = std::process::Command::new("cmd").args(["/C", "start", "", &path.to_string_lossy()]).spawn();
        #[cfg(target_os = "macos")]
        let _ = std::process::Command::new("open").arg(&path).spawn();
        #[cfg(not(any(windows, target_os = "macos")))]
        let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
    });
}
