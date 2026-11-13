//! The agents panel (EXPERIMENTAL, Ctrl+Shift+A): give a task to a coding
//! agent that works in its own worktree and sandbox, watch its log, then
//! review the branch, merge it or discard it. Merge and discard always ask
//! first and say exactly what happens (AGENTS.md rule 8).

use super::*;
use ffwd_agent::{Config as AgentConfig, Event as AgentEvent, EventKind, Record, Status};
use std::collections::VecDeque;

/// One run as the UI knows it.
pub struct AgentRun {
    /// The tab folder the run belongs to.
    pub tab: PathBuf,
    pub rec: Record,
    pub phase: String,
    pub log: VecDeque<String>,
    cap: usize,
    handle: Option<ffwd_agent::Handle>,
    /// Finished while the user was elsewhere: the tab shows a dot.
    pub seen: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Btn {
    Show,
    Merge,
    Discard,
    Cancel,
    Folder,
    Log,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ask {
    Merge,
    Discard,
}

#[derive(Clone, Copy, Debug)]
enum Hit {
    Field,
    Run(usize),
    Button(Btn),
    Confirm(bool),
}

pub struct AgentView {
    input: TextField,
    field: bool,
    sel: usize,
    ask: Option<Ask>,
    /// Lines scrolled up from the end of the log.
    back: usize,
    /// The last result or refusal, shown under the field.
    msg: Option<(bool, String)>,
    hits: Vec<(Rect, Hit)>,
}

impl Ui {
    /// The agent settings of the active tab: its repository's overrides
    /// first, then the global values.
    pub(super) fn agent_config(&self) -> (AgentConfig, Vec<String>) {
        let repo = self.current_repo_settings();
        AgentConfig::from_lookup(|k| {
            let key = format!("agents.{k}");
            repo.and_then(|r| r.get(&key)).or_else(|| self.settings.get(&key)).map(str::to_string)
        })
    }

    /// How many agent settings the active tab's repository overrides.
    pub(super) fn repo_override_count(&self) -> usize {
        self.current_repo_settings().map_or(0, |r| r.keys_with("agents.").len())
    }

    pub(super) fn current_repo_settings(&self) -> Option<&crate::settings::Settings> {
        let t = self.tabs.get(self.cur_tab)?;
        self.repo_settings.get(&repo_key(&t.path))
    }

    /// The active repository's overrides, created (empty, saved to its
    /// file) when they have not loaded yet.
    pub(super) fn current_repo_settings_mut(&mut self) -> Option<&mut crate::settings::Settings> {
        let t = self.tabs.get(self.cur_tab)?;
        let key = repo_key(&t.path);
        let file = crate::settings::repo_settings_path(&t.path)?;
        Some(self.repo_settings.entry(key).or_insert_with(|| crate::settings::Settings::for_file(file)))
    }

    /// Reads the active repository's override file on a worker, once.
    pub(super) fn load_repo_settings(&mut self) {
        let Some(t) = self.tabs.get(self.cur_tab) else { return };
        let key = repo_key(&t.path);
        if self.repo_settings_requested.contains(&key) {
            return;
        }
        let Some(file) = crate::settings::repo_settings_path(&t.path) else { return };
        self.repo_settings_requested.insert(key);
        let path = t.path.clone();
        crate::work::spawn("repo-settings", &(self.sinks)(0), move |sink| {
            sink(Msg::RepoSettings(path, crate::settings::Settings::load_file(file)))
        });
    }

    pub(super) fn on_repo_settings(&mut self, path: PathBuf, loaded: crate::settings::Settings) {
        match self.repo_settings.entry(repo_key(&path)) {
            std::collections::hash_map::Entry::Occupied(mut e) => e.get_mut().fill_from(loaded),
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert(loaded);
            }
        }
        // A repository may turn agents on for itself only.
        if self.tabs.get(self.cur_tab).is_some_and(|t| same_path(&t.path, &path)) {
            self.load_agent_runs();
        }
        self.agents_dirty();
        if self.settings_view.is_some() {
            let r = self.settings_rect();
            self.damage.rects.push(r);
        }
    }

    fn agent_emit(&self) -> ffwd_agent::run::Emit {
        let sink = (self.sinks)(0);
        Arc::new(move |e| sink(Msg::Agent(e)))
    }

    fn tab_path(&self) -> Option<PathBuf> {
        self.tabs.get(self.cur_tab).map(|t| t.path.clone())
    }

    /// Indices into `self.agents` for the active tab, newest first.
    fn tab_runs(&self) -> Vec<usize> {
        let Some(p) = self.tabs.get(self.cur_tab).map(|t| &t.path) else { return Vec::new() };
        let mut v: Vec<usize> = (0..self.agents.len()).filter(|&i| same_path(&self.agents[i].tab, p)).collect();
        v.sort_by_key(|&i| std::cmp::Reverse(self.agents[i].rec.started));
        v
    }

    /// Reads the runs recorded for the active tab once (on a worker).
    pub(super) fn load_agent_runs(&mut self) {
        let Some(p) = self.tab_path() else { return };
        let key = p.to_string_lossy().to_lowercase();
        let (cfg, _) = self.agent_config();
        if !cfg.enabled || !self.agents_loaded.insert(key) {
            return;
        }
        ffwd_agent::run::load(cfg, p, self.agent_emit());
    }

    pub fn open_agents(&mut self) {
        self.palette = None;
        self.close_settings();
        if self.tabs.is_empty() {
            self.set_notice("Open a repository first (Ctrl+O).".into());
            return;
        }
        self.load_agent_runs();
        let runs = self.tab_runs();
        for &i in &runs {
            self.agents[i].seen = true;
        }
        self.agent_view = Some(AgentView {
            input: TextField::default(),
            field: true,
            sel: 0,
            ask: None,
            back: 0,
            msg: None,
            hits: Vec::new(),
        });
        self.damage.full = true;
    }

    pub(super) fn close_agents(&mut self) {
        if self.agent_view.take().is_some() {
            self.damage.full = true;
        }
    }

    pub(super) fn agents_rect(&self) -> Rect {
        let w = self.px(1060.0).min(self.w as i32 - self.px(48.0));
        let h = self.px(740.0).min(self.h as i32 - self.px(48.0));
        Rect::new((self.w as i32 - w) / 2, (self.h as i32 - h) / 2, w, h)
    }

    fn agents_dirty(&mut self) {
        if self.agent_view.is_some() {
            let r = self.agents_rect();
            self.damage.rects.push(r);
        }
    }

    fn agent_msg(&mut self, ok: bool, m: String) {
        if let Some(v) = &mut self.agent_view {
            v.msg = Some((ok, m.clone()));
            self.agents_dirty();
        } else {
            self.set_notice(m);
        }
    }

    fn start_agent(&mut self) {
        let Some(v) = &self.agent_view else { return };
        let prompt = v.input.text().trim().to_string();
        if prompt.is_empty() {
            return;
        }
        let (cfg, _) = self.agent_config();
        if let Some(why) = cfg.refusal() {
            return self.agent_msg(false, why);
        }
        let active = self.agents.iter().filter(|a| a.rec.status.active()).count();
        if active >= cfg.max_parallel {
            return self.agent_msg(
                false,
                format!("{active} agents are running; agents.max_parallel allows {}.", cfg.max_parallel),
            );
        }
        let Some(tab) = self.tab_path() else { return };
        let cap = cfg.log_lines;
        let prefix = cfg.branch_prefix.clone();
        let sandbox = cfg.sandbox.label().to_string();
        let (id, handle) = ffwd_agent::run::start(cfg, tab.clone(), prompt.clone(), self.agent_emit());
        self.agents.push(AgentRun {
            tab: tab.clone(),
            rec: Record {
                id: id.clone(),
                repo: tab,
                worktree: PathBuf::new(),
                branch: format!("{prefix}{id}"),
                base: String::new(),
                title: ffwd_agent::run::title(&prompt),
                started: now_secs(),
                ended: None,
                status: Status::Preparing,
                detail: String::new(),
                commits: 0,
                shortstat: String::new(),
                sandbox,
            },
            phase: "Starting".into(),
            log: VecDeque::new(),
            cap,
            handle: Some(handle),
            seen: true,
        });
        if let Some(v) = &mut self.agent_view {
            v.input = TextField::default();
            v.field = false;
            v.sel = 0;
            v.back = 0;
            v.ask = None;
            v.msg = None;
        }
        self.damage.rects.push(self.lay.tabs);
        self.agents_dirty();
    }

    /// A worker result for an agent run.
    pub(super) fn on_agent_event(&mut self, e: AgentEvent) {
        let tab_is_current = self.tabs.get(self.cur_tab).is_some_and(|t| same_path(&t.path, &e.repo));
        let viewing = self.agent_view.is_some() && tab_is_current;
        let idx = self.agents.iter().position(|a| a.rec.id == e.id && same_path(&a.tab, &e.repo));
        match e.kind {
            EventKind::Loaded(recs) => {
                for rec in recs {
                    if !self.agents.iter().any(|a| a.rec.id == rec.id && same_path(&a.tab, &e.repo)) {
                        self.agents.push(AgentRun {
                            tab: e.repo.clone(),
                            rec,
                            phase: String::new(),
                            log: VecDeque::new(),
                            cap: 5000,
                            handle: None,
                            seen: true,
                        });
                    }
                }
                self.agents_dirty();
            }
            EventKind::Phase(p) => {
                if let Some(i) = idx {
                    self.agents[i].phase = p;
                    self.agents_dirty();
                }
            }
            EventKind::Log(lines) => {
                if let Some(i) = idx {
                    let a = &mut self.agents[i];
                    a.log.extend(lines);
                    while a.log.len() > a.cap.max(100) {
                        a.log.pop_front();
                    }
                    if viewing {
                        self.agents_dirty();
                    }
                }
            }
            EventKind::Update(mut rec) => {
                let Some(i) = idx else { return };
                // The UI keys runs by the tab folder; the record names the
                // repository's top folder, which may differ.
                let was_active = self.agents[i].rec.status.active();
                let now_done = !rec.status.active();
                if rec.repo.as_os_str().is_empty() {
                    rec.repo = self.agents[i].tab.clone();
                }
                if rec.status == Status::Discarded {
                    self.agents.remove(i);
                    let n = self.tab_runs().len();
                    if let Some(v) = &mut self.agent_view {
                        v.sel = v.sel.min(n.saturating_sub(1));
                    }
                    self.agents_dirty();
                    return;
                }
                let title = rec.title.clone();
                let detail = rec.detail.clone();
                // A refused run never made a branch: nothing to reload.
                let made_branch = !rec.worktree.as_os_str().is_empty();
                let word = rec.status.word();
                self.agents[i].rec = *rec;
                if was_active && now_done {
                    self.agents[i].handle = None;
                    self.agents[i].seen = viewing;
                    self.set_notice(format!("Agent “{title}” {word}: {detail}  (Ctrl+Shift+A to review)"));
                    if made_branch {
                        self.refresh_repo(&e.repo);
                    }
                    self.damage.rects.push(self.lay.tabs);
                } else if was_active {
                    // A new branch exists from the moment the worktree does.
                    self.damage.rects.push(self.lay.tabs);
                }
                self.agents_dirty();
            }
            EventKind::ActionDone { ok, message } => {
                self.agent_msg(ok, message.clone());
                self.set_notice(message);
                if ok {
                    self.refresh_repo(&e.repo);
                }
            }
        }
    }

    /// Reloads the repository of `path` now when it is the active tab, or
    /// when that tab is activated next.
    fn refresh_repo(&mut self, path: &Path) {
        let Some(i) = self.tabs.iter().position(|t| same_path(&t.path, path)) else { return };
        if i == self.cur_tab {
            let keep = self.notice.clone();
            self.reload_current();
            if keep.is_some() {
                self.notice = keep;
            }
        } else {
            self.tabs[i].stale = true;
        }
    }

    /// The tab's badge: running (accent), finished unseen (green or red).
    pub(super) fn agent_badge(&self, tab: &Path) -> Option<u32> {
        let mut out = None;
        for a in self.agents.iter().filter(|a| same_path(&a.tab, tab)) {
            if a.rec.status.active() {
                return Some(ACCENT);
            }
            if !a.seen {
                out = Some(if a.rec.status == Status::Done { ADD } else { DEL });
            }
        }
        out
    }

    fn selected_run(&self) -> Option<usize> {
        let v = self.agent_view.as_ref()?;
        self.tab_runs().get(v.sel).copied()
    }

    fn agent_button(&mut self, b: Btn) {
        let Some(i) = self.selected_run() else { return };
        let (cfg, _) = self.agent_config();
        let active = self.agents[i].rec.status.active();
        let rec = self.agents[i].rec.clone();
        let tab = self.agents[i].tab.clone();
        match b {
            Btn::Show => {
                let found = self
                    .r
                    .refs
                    .refs
                    .iter()
                    .find(|r| r.kind == RefKind::Local && *r.name == *rec.branch)
                    .map(|r| r.target);
                match found {
                    Some(id) => {
                        self.close_agents();
                        self.side_select_ref(&rec.branch, RefKind::Local);
                        match self.r.by_id.get(&id) {
                            Some(&row) => self.select(row as usize),
                            None => self.set_notice("That commit is not loaded yet.".into()),
                        }
                    }
                    None => {
                        // The branch is newer than the loaded refs.
                        self.refresh_repo(&tab);
                        self.agent_msg(
                            false,
                            format!("Loading {} into the history; try again in a moment.", rec.branch),
                        );
                    }
                }
            }
            Btn::Merge | Btn::Discard if active => {
                self.agent_msg(false, "The agent is still running; cancel it or wait until it ends.".into())
            }
            Btn::Merge if rec.status == Status::Merged => self.agent_msg(false, "Already merged.".into()),
            Btn::Merge => {
                if let Some(v) = &mut self.agent_view {
                    v.ask = Some(Ask::Merge);
                }
            }
            Btn::Discard => {
                if let Some(v) = &mut self.agent_view {
                    v.ask = Some(Ask::Discard);
                }
            }
            Btn::Cancel => match &self.agents[i].handle {
                Some(h) if active => {
                    h.cancel();
                    self.agent_msg(
                        true,
                        "Cancelling: the agent is stopped; what it changed is committed to its branch.".into(),
                    );
                }
                _ => {}
            },
            Btn::Folder => open_path(rec.worktree.clone()),
            Btn::Log => open_path(
                ffwd_agent::record::run_dir(&ffwd_agent::record::root(&cfg, &rec.repo), &rec.id).join("log.txt"),
            ),
        }
        self.agents_dirty();
    }

    fn agent_confirm(&mut self, yes: bool) {
        let Some(ask) = self.agent_view.as_mut().and_then(|v| v.ask.take()) else { return };
        self.agents_dirty();
        if !yes {
            return;
        }
        let Some(i) = self.selected_run() else { return };
        let (cfg, _) = self.agent_config();
        let rec = self.agents[i].rec.clone();
        let tab = self.agents[i].tab.clone();
        let emit = self.agent_emit();
        match ask {
            Ask::Merge => ffwd_agent::run::merge(cfg, tab, rec, emit),
            Ask::Discard => ffwd_agent::run::discard(cfg, tab, rec, emit),
        }
        self.agent_msg(true, "Working…".into());
    }

    fn ask_text(&self, ask: Ask, rec: &Record) -> String {
        let n = |c: u32| format!("{c} commit{}", if c == 1 { "" } else { "s" });
        match ask {
            Ask::Merge => format!(
                "Merge {} ({}) into {}? Git merges with --no-ff. If conflicts appear the merge stops, and git merge --abort undoes it.",
                rec.branch,
                n(rec.commits),
                self.r.refs.head_branch.as_deref().unwrap_or("the current branch")
            ),
            Ask::Discard => format!(
                "Discard {}? This deletes the worktree {} with any uncommitted files in it, and deletes the branch ({}). ffwd then shows the command that restores the branch.",
                rec.branch,
                rec.worktree.display(),
                n(rec.commits)
            ),
        }
    }

    pub(super) fn agents_key(&mut self, k: Key) {
        let Some(v) = &mut self.agent_view else { return };
        if v.ask.is_some() {
            match k {
                Key::Enter | Key::Char('y') => self.agent_confirm(true),
                Key::Esc | Key::Char('n') => self.agent_confirm(false),
                _ => {}
            }
            return;
        }
        let n = self.tab_runs().len();
        let Some(v) = &mut self.agent_view else { return };
        if v.field {
            let f = &mut v.input;
            match k {
                Key::Esc if !f.text().is_empty() => *f = TextField::default(),
                Key::Esc | Key::CtrlShift('a') => return self.close_agents(),
                Key::Enter => return self.start_agent(),
                Key::Down | Key::Tab if n > 0 => v.field = false,
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
                Key::Ctrl('x') => {
                    let t = f.cut();
                    crate::platform::set_clipboard(&t);
                }
                Key::Ctrl('v') => {
                    if let Some(t) = crate::platform::clipboard() {
                        f.insert(&t);
                    }
                }
                _ => {}
            }
            self.agents_dirty();
            return;
        }
        match k {
            Key::Esc | Key::CtrlShift('a') => return self.close_agents(),
            Key::Up if v.sel == 0 => v.field = true,
            Key::Tab | Key::BackTab => v.field = true,
            Key::Up | Key::Char('k') => {
                v.sel -= 1;
                v.back = 0;
            }
            Key::Down | Key::Char('j') => {
                v.sel = (v.sel + 1).min(n.saturating_sub(1));
                v.back = 0;
            }
            Key::PageUp => v.back += 10,
            Key::PageDown => v.back = v.back.saturating_sub(10),
            Key::Enter => return self.agent_button(Btn::Show),
            Key::Char('m') => return self.agent_button(Btn::Merge),
            Key::Char('d') | Key::Delete => return self.agent_button(Btn::Discard),
            Key::Char('c') => return self.agent_button(Btn::Cancel),
            Key::Char('o') => return self.agent_button(Btn::Folder),
            Key::Char('l') => return self.agent_button(Btn::Log),
            Key::Char(c) if !c.is_whitespace() => {
                // Typing starts a new task.
                v.field = true;
                v.input.insert(c.encode_utf8(&mut [0; 4]));
            }
            _ => {}
        }
        self.agents_dirty();
    }

    pub(super) fn agents_click(&mut self, x: i32, y: i32) {
        let r = self.agents_rect();
        if !r.contains(x, y) {
            return self.close_agents();
        }
        let hit = self.agent_view.as_ref().and_then(|v| v.hits.iter().find(|h| h.0.contains(x, y)).map(|h| h.1));
        match hit {
            Some(Hit::Field) => {
                if let Some(v) = &mut self.agent_view {
                    v.field = true;
                }
            }
            Some(Hit::Run(i)) => {
                if let Some(v) = &mut self.agent_view {
                    v.field = false;
                    v.sel = i;
                    v.back = 0;
                    v.ask = None;
                }
            }
            Some(Hit::Button(b)) => self.agent_button(b),
            Some(Hit::Confirm(yes)) => self.agent_confirm(yes),
            None => {}
        }
        self.agents_dirty();
    }

    pub(super) fn agents_wheel(&mut self, lines: i64) {
        if let Some(v) = &mut self.agent_view {
            v.back = (v.back as i64 - lines).max(0) as usize;
            self.agents_dirty();
        }
    }

    /// The index of the panel element under the pointer (for hover).
    pub(super) fn agent_hit_at(&self, x: i32, y: i32) -> Option<usize> {
        self.agent_view.as_ref()?.hits.iter().position(|h| h.0.contains(x, y))
    }

    pub(super) fn agent_hit_is_field(&self, i: usize) -> bool {
        self.agent_view.as_ref().is_some_and(|v| matches!(v.hits.get(i), Some((_, Hit::Field))))
    }

    fn pill(&mut self, cv: &mut Canvas, x: f32, cy: f32, text: &str, bg: u32, fg: u32) -> f32 {
        let semi = self.f.semi;
        let size = self.pf(10.0);
        let w = self.te.measure(semi, size, text) + self.pf(14.0);
        let r = Rect::new(x as i32, (cy - self.pf(9.0)) as i32, w as i32, self.px(18.0));
        cv.fill_round(r, self.pf(9.0), bg);
        self.te.draw(cv, semi, size, x + self.pf(7.0), (cy + self.pf(3.5)).round(), text, fg, w);
        x + w
    }

    /// A panel button; `hit` is its index in the hit list being built.
    #[allow(clippy::too_many_arguments)]
    fn agent_btn(
        &mut self,
        cv: &mut Canvas,
        x: i32,
        y: i32,
        label: &str,
        enabled: bool,
        danger: bool,
        hit: usize,
    ) -> Rect {
        let semi = self.f.semi;
        let w = (self.te.measure(semi, self.pf(12.0), label) + self.pf(24.0)) as i32;
        let r = Rect::new(x, y, w, self.px(28.0));
        let h = Hot::Agent(hit);
        let (border, bg, fg) = match (enabled, danger) {
            (false, _) => (BORDER_SOFT, BG_PANEL, TEXT_4),
            (true, true) if self.is_hot(h) => (DEL, DEL_BG, DEL_TEXT),
            (true, true) => (DANGER_BORDER, DEL_BG, DEL_TEXT),
            (true, false) => {
                let (bg, border) = self.control_colors(h, BG_BAR, BORDER);
                (border, bg, TEXT)
            }
        };
        cv.round_border(r, self.pf(7.0), border, bg);
        self.te.draw(cv, semi, self.pf(12.0), x as f32 + self.pf(12.0), y as f32 + self.pf(18.5), label, fg, w as f32);
        r
    }

    pub(super) fn paint_agents(&mut self, cv: &mut Canvas) {
        let Some(mut view) = self.agent_view.take() else { return };
        let r = self.agents_rect();
        let old = cv.set_clip(cv.clip().intersect(r));
        cv.round_border(r, self.pf(12.0), BORDER_STRONG, BG_PANEL);
        let (ui, semi, mono) = (self.f.ui, self.f.semi, self.f.mono);
        let pad = self.px(24.0);
        let x0 = r.x + pad;
        let inner_w = r.w - 2 * pad;
        let mut hits = Vec::new();
        let (cfg, cfg_errors) = self.agent_config();

        // Title row.
        let ty = r.y as f32 + self.pf(38.0);
        let e = self.te.draw(cv, semi, self.pf(18.0), x0 as f32, ty, "Agents", TEXT, self.pf(200.0));
        let e = self.pill(cv, e + self.pf(12.0), ty - self.pf(6.0), "EXPERIMENTAL", TAG_BG, TAG_FG);
        self.te.draw(cv, ui, self.pf(12.5), e + self.pf(12.0), ty, &self.r.repo_name.clone(), TEXT_3, self.pf(260.0));
        let hint = "Esc closes · Ctrl+, → Agents for settings";
        let hw = self.te.measure(ui, self.pf(11.5), hint);
        self.te.draw(cv, ui, self.pf(11.5), r.right() as f32 - pad as f32 - hw, ty, hint, TEXT_4, hw + 2.0);

        // Isolation line: what the agent can reach, in one sentence.
        let sy = ty + self.pf(24.0);
        let refusal = cfg.refusal();
        let (line, col) = match (&refusal, cfg.sandbox.resolve()) {
            (Some(why), _) => (why.clone(), MOD),
            (None, ffwd_agent::SandboxKind::None) => (cfg.summary(), DEL),
            (None, _) => (cfg.summary(), TEXT_3),
        };
        let overrides = self.repo_override_count();
        let line = if overrides > 0 {
            format!("{line} · {overrides} setting{} for this repository", if overrides == 1 { "" } else { "s" })
        } else {
            line
        };
        let line = match cfg_errors.first() {
            Some(err) => format!("{line} · settings: {err}"),
            None => line,
        };
        self.te.draw(cv, ui, self.pf(12.0), x0 as f32, sy, &line, col, inner_w as f32);

        // Prompt field.
        let fr = Rect::new(x0, (sy + self.pf(14.0)) as i32, inner_w, self.px(38.0));
        cv.round_border(fr, self.pf(8.0), if view.field { ACCENT } else { BORDER }, BG_INPUT);
        let fx = fr.x as f32 + self.pf(14.0);
        let fb = (fr.y as f32 + fr.h as f32 / 2.0 + self.pf(4.5)).round();
        let fmax = fr.w as f32 - self.pf(28.0);
        if view.input.text().is_empty() {
            let ph = "Describe a task. Enter starts an agent on a new branch, in its own worktree.";
            self.te.draw(cv, ui, self.pf(13.0), fx, fb, ph, TEXT_4, fmax);
        } else {
            if view.input.all_selected() {
                let w = self.te.measure(ui, self.pf(13.0), view.input.text()).min(fmax);
                cv.fill_round(
                    Rect::new(fx as i32 - 2, fr.y + self.px(8.0), w as i32 + 4, fr.h - self.px(16.0)),
                    self.pf(3.0),
                    BG_TEXT_SELECT,
                );
            }
            self.te.draw(cv, ui, self.pf(13.0), fx, fb, view.input.text(), TEXT, fmax);
        }
        if view.field {
            let cx = fx + self.te.measure(ui, self.pf(13.0), &view.input.text()[..view.input.cursor()]);
            cv.fill(Rect::new(cx as i32 + 1, fr.y + self.px(9.0), self.px(1.5).max(1), fr.h - self.px(18.0)), ACCENT);
        }
        hits.push((fr, Hit::Field));
        let mut y = fr.bottom() + self.px(10.0);
        if let Some((ok, m)) = &view.msg {
            self.te.draw(
                cv,
                ui,
                self.pf(12.0),
                x0 as f32,
                y as f32 + self.pf(10.0),
                m,
                if *ok { TEXT_2 } else { MOD },
                inner_w as f32,
            );
            y += self.px(20.0);
        }
        y += self.px(6.0);

        // Run list (left) and the selected run (right).
        let runs = self.tab_runs();
        view.sel = view.sel.min(runs.len().saturating_sub(1));
        let body = Rect::new(x0, y, inner_w, r.bottom() - pad - y);
        let list_w = (body.w as f32 * 0.34) as i32;
        let list = Rect::new(body.x, body.y, list_w, body.h);
        cv.round_border(list, self.pf(8.0), BORDER_SOFT, BG_SIDEBAR);
        let row_h = self.px(54.0);
        if runs.is_empty() {
            let lines = [
                "No agent runs in this repository yet.",
                "",
                "Each run gets its own worktree and branch",
                "next to the repository, in a sandbox.",
                "Your checkout is never touched: you review",
                "the branch, then merge or discard it.",
            ];
            for (k, l) in lines.iter().enumerate() {
                let c = if k == 0 { TEXT_2 } else { TEXT_3 };
                self.te.draw(
                    cv,
                    ui,
                    self.pf(12.0),
                    list.x as f32 + self.pf(16.0),
                    list.y as f32 + self.pf(28.0) + k as f32 * self.pf(19.0),
                    l,
                    c,
                    list.w as f32 - self.pf(32.0),
                );
            }
        }
        for (k, &i) in runs.iter().enumerate() {
            let ry = list.y + self.px(6.0) + k as i32 * row_h;
            if ry + row_h > list.bottom() {
                break;
            }
            let rr = Rect::new(list.x + self.px(6.0), ry, list.w - self.px(12.0), row_h - self.px(4.0));
            if k == view.sel {
                cv.fill_round(rr, self.pf(7.0), if view.field { BG_SELECT_DIM } else { BG_SELECT });
            } else if self.is_hot(Hot::Agent(hits.len())) {
                cv.fill_round(rr, self.pf(7.0), BG_HOVER);
            }
            let a = &self.agents[i];
            let (title, status, branch) = (a.rec.title.clone(), a.rec.status, a.rec.branch.clone());
            let phase = a.phase.clone();
            let dot = status_color(status);
            cv.fill_circle(rr.x as f32 + self.pf(14.0), rr.y as f32 + self.pf(18.0), self.pf(4.0), dot);
            let tx = rr.x as f32 + self.pf(28.0);
            let tw = rr.w as f32 - self.pf(36.0);
            self.te.draw(cv, semi, self.pf(13.0), tx, rr.y as f32 + self.pf(22.0), &title, TEXT, tw);
            let word = if status.active() && !phase.is_empty() {
                format!("{} · {}", status.word(), phase.to_lowercase())
            } else {
                status.word().into()
            };
            let e = self.te.draw(cv, ui, self.pf(11.5), tx, rr.y as f32 + self.pf(40.0), &word, dot, tw);
            self.te.draw(
                cv,
                mono,
                self.pf(11.0),
                e + self.pf(8.0),
                rr.y as f32 + self.pf(40.0),
                &branch,
                TEXT_3,
                tx + tw - e - self.pf(8.0),
            );
            hits.push((rr, Hit::Run(k)));
        }

        // Details of the selected run.
        let dx = list.right() + self.px(18.0);
        let dw = body.right() - dx;
        if let Some(&i) = runs.get(view.sel) {
            let a = &self.agents[i];
            let rec = a.rec.clone();
            let has_handle = a.handle.is_some();
            let mut yy = body.y as f32 + self.pf(16.0);
            self.te.draw(cv, semi, self.pf(15.0), dx as f32, yy, &rec.title, TEXT, dw as f32);
            yy += self.pf(22.0);
            let base = if rec.base.is_empty() {
                String::new()
            } else {
                format!(" from {}", &rec.base[..rec.base.len().min(8)])
            };
            let e =
                self.te.draw(cv, mono, self.pf(12.0), dx as f32, yy, &format!("{}{base}", rec.branch), REN, dw as f32);
            let when = fmt::relative(rec.started, now_secs(), self.r.tz);
            self.te.draw(
                cv,
                ui,
                self.pf(12.0),
                e + self.pf(10.0),
                yy,
                &format!("· started {when} · {}", rec.sandbox),
                TEXT_3,
                dx as f32 + dw as f32 - e,
            );
            yy += self.pf(20.0);
            let status_line = if rec.detail.is_empty() {
                rec.status.word().to_string()
            } else {
                format!("{} · {}", rec.status.word(), rec.detail)
            };
            self.te.draw(cv, semi, self.pf(12.5), dx as f32, yy, &status_line, status_color(rec.status), dw as f32);
            yy += self.pf(18.0);
            if !rec.worktree.as_os_str().is_empty() {
                self.te.draw(
                    cv,
                    mono,
                    self.pf(11.0),
                    dx as f32,
                    yy,
                    &rec.worktree.to_string_lossy(),
                    TEXT_4,
                    dw as f32,
                );
            }
            yy += self.pf(14.0);
            // Buttons.
            let active = rec.status.active();
            let finished = !active && rec.status != Status::Merged;
            let mut bx = dx;
            let by = yy as i32;
            for (b, label, en, danger) in [
                (Btn::Show, "Show branch", !rec.worktree.as_os_str().is_empty(), false),
                (Btn::Merge, "Merge…", finished && rec.commits > 0, false),
                (Btn::Discard, "Discard…", !active, true),
                (Btn::Cancel, "Cancel run", active && has_handle, true),
                (Btn::Folder, "Open folder", !rec.worktree.as_os_str().is_empty(), false),
                (Btn::Log, "Open log", !rec.worktree.as_os_str().is_empty(), false),
            ] {
                let br = self.agent_btn(cv, bx, by, label, en, danger, hits.len());
                if en {
                    hits.push((br, Hit::Button(b)));
                }
                bx = br.right() + self.px(8.0);
            }
            let mut ly = by + self.px(40.0);
            // Confirmation.
            if let Some(ask) = view.ask {
                let text = self.ask_text(ask, &rec);
                let (sz, maxw) = (self.pf(12.5), dw as f32 - self.pf(28.0));
                let lines = wrap(&mut self.te, ui, sz, &text, maxw);
                let h = self.px(20.0) + lines.len() as i32 * self.px(18.0) + self.px(44.0);
                let cr = Rect::new(dx, ly, dw, h);
                let danger = ask == Ask::Discard;
                cv.round_border(
                    cr,
                    self.pf(8.0),
                    if danger { DANGER_BORDER } else { HUNK_BORDER },
                    if danger { DEL_BG } else { HUNK_BG },
                );
                for (k, l) in lines.iter().enumerate() {
                    self.te.draw(
                        cv,
                        ui,
                        self.pf(12.5),
                        cr.x as f32 + self.pf(14.0),
                        cr.y as f32 + self.pf(24.0) + k as f32 * self.pf(18.0),
                        l,
                        TEXT,
                        cr.w as f32,
                    );
                }
                let byy = cr.bottom() - self.px(38.0);
                let yes = if danger { "Discard (Enter)" } else { "Merge (Enter)" };
                let b1 = self.agent_btn(cv, cr.x + self.px(14.0), byy, yes, true, danger, hits.len());
                hits.push((b1, Hit::Confirm(true)));
                let b2 = self.agent_btn(cv, b1.right() + self.px(8.0), byy, "Keep it (Esc)", true, false, hits.len());
                hits.push((b2, Hit::Confirm(false)));
                ly = cr.bottom() + self.px(10.0);
            }
            // Log tail.
            let lr = Rect::new(dx, ly, dw, body.bottom() - ly);
            if lr.h > self.px(30.0) {
                cv.round_border(lr, self.pf(8.0), BORDER_SOFT, BG_SIDEBAR);
                let lh = self.px(17.0);
                let fit = ((lr.h - self.px(16.0)) / lh).max(0) as usize;
                let log = &self.agents[i].log;
                if log.is_empty() {
                    let t = if active {
                        "Waiting for output…"
                    } else {
                        "No output in this session; Open log shows the saved log."
                    };
                    self.te.draw(
                        cv,
                        ui,
                        self.pf(12.0),
                        lr.x as f32 + self.pf(12.0),
                        lr.y as f32 + self.pf(22.0),
                        t,
                        TEXT_4,
                        lr.w as f32,
                    );
                }
                let total = log.len();
                view.back = view.back.min(total.saturating_sub(fit));
                let end = total - view.back;
                let start = end.saturating_sub(fit);
                let lines: Vec<String> = log.range(start..end).cloned().collect();
                for (k, l) in lines.iter().enumerate() {
                    let c = if l.starts_with("» ") { HUNK_TEXT } else { TEXT_2 };
                    self.te.draw(
                        cv,
                        mono,
                        self.pf(11.5),
                        lr.x as f32 + self.pf(12.0),
                        lr.y as f32 + self.pf(20.0) + (k as i32 * lh) as f32,
                        l,
                        c,
                        lr.w as f32 - self.pf(24.0),
                    );
                }
                if view.back > 0 {
                    let t = format!("↓ {} newer lines (PageDown)", view.back);
                    let w = self.te.measure(ui, self.pf(11.0), &t);
                    self.te.draw(
                        cv,
                        ui,
                        self.pf(11.0),
                        lr.right() as f32 - w - self.pf(12.0),
                        lr.y as f32 + self.pf(18.0),
                        &t,
                        TEXT_3,
                        w + 2.0,
                    );
                }
            }
        } else {
            let t = "Runs appear here with their log. Keys: ↑↓ select · Enter show branch · m merge · d discard · c cancel · o folder · l log";
            let sz = self.pf(12.0);
            for (k, l) in wrap(&mut self.te, ui, sz, t, dw as f32).iter().enumerate() {
                self.te.draw(
                    cv,
                    ui,
                    self.pf(12.0),
                    dx as f32,
                    body.y as f32 + self.pf(22.0) + k as f32 * self.pf(18.0),
                    l,
                    TEXT_3,
                    dw as f32,
                );
            }
        }
        cv.set_clip(old);
        view.hits = hits;
        self.agent_view = Some(view);
    }

    /// Snapshot only: Settings → Agents in "This repository" scope with
    /// example overrides held in memory (nothing is written).
    pub fn snapshot_repo_scope_demo(&mut self) {
        let Some(t) = self.tabs.get(self.cur_tab) else { return };
        let key = repo_key(&t.path);
        let demo = "[agents]
enabled = \"true\"
preset = \"codex\"
sandbox = \"none\"
allow_unsandboxed = \"true\"
setup_script = \"cargo fetch\"
";
        self.repo_settings.insert(key.clone(), crate::settings::Settings::parse(demo));
        self.repo_settings_requested.insert(key);
        self.open_settings_page(1);
        self.agent_scope_repo = true;
    }

    /// Snapshot only: the agent's per-path access list (FF-23), scrolled to
    /// the add row. In-memory settings; nothing is written.
    pub fn snapshot_access_demo(&mut self) {
        let Some(t) = self.tabs.get(self.cur_tab) else { return };
        let key = repo_key(&t.path);
        let demo = "[agents]
enabled = \"true\"
preset = \"claude\"
preset_access = \"~/.claude.json=read\"
read_paths = \"~/.gitconfig\"
write_paths = \"/data/shared\"
";
        self.repo_settings.insert(key.clone(), crate::settings::Settings::parse(demo));
        self.repo_settings_requested.insert(key);
        self.open_settings_page(1);
        self.agent_scope_repo = true;
        let add = self.rows().iter().position(|r| *r == settings_view::Row::AddGrant).unwrap_or(0);
        if let Some(v) = &mut self.settings_view {
            v.sel = add;
        }
        self.settings_follow();
    }

    /// Snapshot only: example runs, so the panel can be checked offscreen.
    pub fn snapshot_agents_demo(&mut self) {
        let Some(tab) = self.tab_path() else { return };
        // Snapshot settings live in memory only (no file), so this writes
        // nothing to disk.
        self.settings.set_string("agents.enabled", "true");
        self.settings.set_string("agents.command", "aider --yes-always --message-file {prompt_file}");
        self.settings.set_string("agents.sandbox", "wsl");
        let now = now_secs();
        let mk = |id: &str, title: &str, status: Status, detail: &str, commits: u32, ago: i64| Record {
            id: id.into(),
            repo: tab.clone(),
            worktree: tab.parent().unwrap_or(&tab).join(".ffwd-agents").join("ffwd").join(id),
            branch: format!("agent/{id}"),
            base: "f196dc3a0b1c".into(),
            title: title.into(),
            started: now - ago,
            ended: None,
            status,
            detail: detail.into(),
            commits,
            shortstat: String::new(),
            sandbox: "WSL + bubblewrap".into(),
        };
        let runs = [
            (
                mk("add-blame-view-k3x9a", "Add a blame view for the selected file", Status::Running, "", 0, 240),
                "Agent",
            ),
            (
                mk(
                    "fix-tab-close-q1w2e",
                    "Fix the tab close button hit area",
                    Status::Done,
                    "2 commits · 3 files changed, 41 insertions(+), 9 deletions(-)",
                    2,
                    3600,
                ),
                "",
            ),
            (
                mk(
                    "speed-up-refs-z8y7x",
                    "Speed up the refs loader",
                    Status::TimedOut,
                    "agent: stopped at the time limit",
                    1,
                    7200 * 3,
                ),
                "",
            ),
        ];
        for (rec, phase) in runs {
            let mut log = VecDeque::new();
            if rec.status == Status::Running {
                for l in [
                    "» ffwd agent run add-blame-view-k3x9a (experimental) · Sandbox: WSL + bubblewrap · network on · Git read-only (ffwd commits) · 60 min limit",
                    "» Creating worktree",
                    "» worktree E:\\Development\\.ffwd-agents\\ffwd\\add-blame-view-k3x9a on agent/add-blame-view-k3x9a from f196dc3a0b",
                    "» Setup script",
                    "» sh -c \"cargo fetch\"",
                    "» Setup script: exit code 0",
                    "» Agent",
                    "» wsl.exe --cd /mnt/e/Development/.ffwd-agents/ffwd/add-blame-view-k3x9a -e sh -c \"exec \\\"$0\\\" \\\"$@\\\"\" bwrap …",
                    "Reading crates/ffwd-core/src/diff.rs",
                    "Reading crates/ffwd-app/src/ui.rs",
                    "Plan: add blame::stream in ffwd-core, a worker in work.rs, a virtualised view in ui_blame.rs",
                    "Editing crates/ffwd-core/src/blame.rs (new file)",
                    "Running cargo test -p ffwd-core blame",
                    "test blame::tests::parses_porcelain ... ok",
                ] {
                    log.push_back(l.to_string());
                }
            }
            self.agents.push(AgentRun {
                tab: tab.clone(),
                seen: rec.status != Status::Done,
                rec,
                phase: phase.into(),
                log,
                cap: 5000,
                handle: Some(ffwd_agent::Handle::default()),
            });
        }
        // The finished-but-unseen dot is what the tab shows; opening the
        // panel marks the runs seen, so open it without that step.
        self.load_agent_runs();
        self.agent_view = Some(AgentView {
            input: TextField::new("Add keyboard shortcuts to the agents panel"),
            field: false,
            sel: 0,
            ask: None,
            back: 0,
            msg: None,
            hits: Vec::new(),
        });
        self.damage.full = true;
    }
}

fn status_color(s: Status) -> u32 {
    match s {
        Status::Preparing | Status::Running | Status::Finishing => ACCENT,
        Status::Done => ADD,
        Status::Merged => REN,
        Status::Cancelled | Status::Interrupted => TEXT_3,
        Status::Failed | Status::TimedOut => DEL,
        Status::Discarded => TEXT_4,
    }
}

/// Opens a folder or file with the system's default program, on a worker.
fn open_path(p: PathBuf) {
    if p.as_os_str().is_empty() {
        return;
    }
    std::thread::spawn(move || {
        #[cfg(windows)]
        let _ = {
            use std::os::windows::process::CommandExt;
            std::process::Command::new("explorer").arg(&p).creation_flags(0x0800_0000).spawn()
        };
        #[cfg(target_os = "macos")]
        let _ = std::process::Command::new("open").arg(&p).spawn();
        #[cfg(not(any(windows, target_os = "macos")))]
        let _ = std::process::Command::new("xdg-open").arg(&p).spawn();
    });
}
