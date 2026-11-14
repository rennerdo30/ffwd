//! The agents page (EXPERIMENTAL, Ctrl+Shift+A, or Agents in the sidebar):
//! give a task to a coding agent that works in its own worktree and
//! sandbox, watch it, then review its changes and merge or discard them
//! (FF-23, FF-57). The page has a rail on the left (a composer, then the
//! runs grouped as running, to review, failed and finished) and the
//! selected run on the right, with the actions that fit its state. Merge
//! and discard always ask first and say exactly what happens (AGENTS.md
//! rule 8).

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
    /// The branch's changes against its base, in the history's compare.
    Review,
    Merge,
    Discard,
    Cancel,
    Folder,
    Log,
    /// The tool's CLI in the run's worktree.
    Terminal,
    /// The task back in the composer.
    Again,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ask {
    Merge,
    Discard,
}

#[derive(Clone, Copy, Debug)]
enum Hit {
    Field,
    /// Start the task.
    Start,
    /// Which agent runs it (a menu).
    Tool,
    /// Open the tool's CLI in a terminal window (a menu of places).
    Terminal,
    Run(usize),
    Button(Btn),
    Confirm(bool),
    /// Output or Details.
    Tab(u8),
    /// The agent settings (header chip and button).
    Settings,
    Close,
    TurnOn,
    /// An example task fills the composer.
    Example(usize),
}

/// Example tasks for an empty page.
const EXAMPLES: [&str; 3] = [
    "Write a README that explains how to build and run this project",
    "Add tests for the module with the fewest tests",
    "Find one bug, fix it, and explain the fix in the commit message",
];

/// The groups of the rail, in order.
const GROUPS: [&str; 4] = ["Running", "To review", "Failed", "Finished"];

fn group_of(rec: &Record) -> usize {
    match rec.status {
        s if s.active() => 0,
        Status::Done if rec.commits > 0 => 1,
        Status::Failed | Status::TimedOut => 2,
        _ => 3,
    }
}

pub struct AgentView {
    input: TextField,
    field: bool,
    sel: usize,
    ask: Option<Ask>,
    /// Lines scrolled up from the end of the log.
    back: usize,
    /// The last result or refusal, shown under the composer.
    msg: Option<(bool, String)>,
    hits: Vec<(Rect, Hit)>,
    /// 0 output, 1 details.
    tab: u8,
    /// Another agent than the configured one, for the next task.
    tool: Option<&'static str>,
    /// The rail scrolled up, in pixels.
    rail_scroll: i32,
    /// Keys go to the page (else the sidebar has them, after a click there).
    focused: bool,
}

impl AgentView {
    fn new(input: &str, field: bool) -> AgentView {
        AgentView {
            input: TextField::new(input),
            field,
            sel: 0,
            ask: None,
            back: 0,
            msg: None,
            hits: Vec::new(),
            tab: 0,
            tool: None,
            rail_scroll: 0,
            focused: true,
        }
    }
}

impl Ui {
    /// The agent settings of the active tab: its repository's overrides
    /// first, then the global values.
    pub(super) fn agent_config(&self) -> (AgentConfig, Vec<String>) {
        self.agent_config_for(None)
    }

    /// [`agent_config`] for another agent (FF-23): its preset's own
    /// command and grants, the rest (sandbox, paths) as configured.
    pub(super) fn agent_config_for(&self, preset: Option<&str>) -> (AgentConfig, Vec<String>) {
        let repo = self.current_repo_settings();
        AgentConfig::from_lookup(|k| {
            if let Some(p) = preset {
                match k {
                    "preset" => return Some(p.to_string()),
                    "command" | "interactive_command" | "preset_access" => return None,
                    _ => {}
                }
            }
            let key = format!("agents.{k}");
            repo.and_then(|r| r.get(&key)).or_else(|| self.settings.get(&key)).map(str::to_string)
        })
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

    /// The active tab's runs as the rail shows them: by group (running, to
    /// review, failed, finished), newest first in each.
    fn ordered_runs(&self) -> Vec<usize> {
        let mut v = self.tab_runs();
        v.sort_by_key(|&i| (group_of(&self.agents[i].rec), std::cmp::Reverse(self.agents[i].rec.started)));
        v
    }

    /// How many runs wait: running, and finished with changes to review.
    pub(super) fn agent_counts(&self) -> (usize, usize) {
        let runs = self.tab_runs();
        let running = runs.iter().filter(|&&i| group_of(&self.agents[i].rec) == 0).count();
        let review = runs.iter().filter(|&&i| group_of(&self.agents[i].rec) == 1).count();
        (running, review)
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
        self.side_select("agents");
        let runs = self.tab_runs();
        for &i in &runs {
            self.agents[i].seen = true;
        }
        self.agent_view = Some(AgentView::new("", true));
        self.damage.full = true;
    }

    pub(super) fn close_agents(&mut self) {
        if self.agent_view.take().is_some() {
            self.damage.full = true;
            // The sidebar names what shows again.
            if self.r.side_sel == "agents" {
                let key = if self.r.changes_view {
                    "changes"
                } else if self.r.files_view {
                    "files"
                } else if self.r.activity_view {
                    "activity"
                } else if self.r.reflog_view {
                    "reflog"
                } else {
                    "all"
                };
                self.side_select(key);
            }
        }
    }

    /// Do keys go to the agents page?
    pub(super) fn agents_have_keys(&self) -> bool {
        self.agent_view.as_ref().is_some_and(|v| v.focused)
    }

    /// The sidebar was clicked while the page shows: keys go there.
    pub(super) fn agents_blur(&mut self) {
        if let Some(v) = &mut self.agent_view {
            v.focused = false;
        }
        self.agents_dirty();
    }

    /// The agents page: a view of the repository like Local Changes,
    /// right of the sidebar and under the tabs (FF-57).
    pub(super) fn agents_rect(&self) -> Rect {
        let s = self.lay.side;
        let top = self.lay.list.y;
        Rect::new(s.right(), top, (self.w as i32 - s.right()).max(1), (self.lay.status.y - top).max(1))
    }

    pub(super) fn agents_rail(&self) -> Rect {
        let r = self.agents_rect();
        let head = self.px(64.0);
        let w = match self.split_get(Split::AgentsRail) {
            Some(v) => self.px(v).max(self.px(260.0)).min((r.w - self.px(360.0)).max(self.px(260.0))),
            None => self.px(360.0).min(r.w / 2),
        };
        Rect::new(r.x, r.y + head, w, r.h - head)
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
        let tool = self.agent_view.as_ref().and_then(|v| v.tool);
        let (cfg, _) = self.agent_config_for(tool);
        self.launch_agent_with(cfg, prompt, ffwd_agent::run::Mode::Prompt);
    }

    /// The agents to pick from for the next task: the configured one
    /// first, then every other built-in one.
    fn agent_tool_menu(&mut self, at: Rect) {
        let now = self.agent_view.as_ref().and_then(|v| v.tool);
        let (cfg, _) = self.agent_config();
        let configured = cfg.preset.map_or_else(|| "Custom command".to_string(), |p| p.name.to_string());
        let mark = |on: bool, t: String| if on { format!("✓ {t}") } else { t };
        let mut items = vec![menu::MenuItem {
            label: mark(now.is_none(), format!("{configured} (from settings)")),
            act: Some(menu::MenuAct::AgentTool(None)),
            danger: false,
        }];
        for p in ffwd_agent::presets::PRESETS.iter().filter(|p| Some(p.id) != cfg.preset.map(|c| c.id)) {
            items.push(menu::MenuItem {
                label: mark(now == Some(p.id), p.name.to_string()),
                act: Some(menu::MenuAct::AgentTool(Some(p.id))),
                danger: false,
            });
        }
        self.open_menu(at.x, at.bottom() + self.px(4.0), items);
    }

    pub(super) fn set_agent_tool(&mut self, tool: Option<&'static str>) {
        if let Some(v) = &mut self.agent_view {
            v.tool = tool;
            v.field = true;
        }
        self.agents_dirty();
    }

    /// The menu of terminal sessions (FF-23): the tool's interactive CLI
    /// in a new worktree (with `worktrees`) or in `path` (None: this
    /// repository), with the sandbox and without.
    pub(super) fn agent_terminal_menu(&self, path: Option<PathBuf>, worktrees: bool) -> Vec<menu::MenuItem> {
        let mut v = self.agent_terminal_items(None, path.clone(), worktrees, true);
        // Every other agent too, with the configured sandbox (FF-23).
        let current = self.agent_config().0.preset.map(|p| p.id);
        let mut first = true;
        for p in ffwd_agent::presets::PRESETS.iter().filter(|p| Some(p.id) != current) {
            if std::mem::take(&mut first) && !v.is_empty() {
                v.push(menu::MenuItem { label: String::new(), act: None, danger: false });
            }
            v.extend(self.agent_terminal_items(Some(p.id), path.clone(), worktrees, false));
        }
        v
    }

    /// The menu items of one agent: in a new worktree (with `worktrees`)
    /// and here, sandboxed; with `both`, also without the sandbox.
    fn agent_terminal_items(
        &self,
        preset: Option<&'static str>,
        path: Option<PathBuf>,
        worktrees: bool,
        both: bool,
    ) -> Vec<menu::MenuItem> {
        let (cfg, _) = self.agent_config_for(preset);
        let tool = cfg.preset.map(|p| p.name.to_string()).unwrap_or_else(|| {
            ffwd_agent::config::split_command(&cfg.interactive_command)
                .ok()
                .and_then(|v| v.into_iter().next())
                .unwrap_or_else(|| "the agent".into())
        });
        let kind = cfg.sandbox.resolve();
        let boxed = kind != ffwd_agent::SandboxKind::None;
        let plain = if kind == ffwd_agent::SandboxKind::Wsl { "WSL, no sandbox" } else { "no sandbox" };
        let here = if path.is_some() { "in this worktree" } else { "in this repository" };
        let it = |label: String, worktree: bool, sandboxed: bool| menu::MenuItem {
            label,
            act: Some(menu::MenuAct::AgentTerminal { path: path.clone(), worktree, sandboxed, preset }),
            danger: false,
        };
        let mut v = Vec::new();
        for sandboxed in [true, false] {
            if (sandboxed && !boxed) || (!sandboxed && !both && boxed) {
                continue;
            }
            let how = if sandboxed { cfg.sandbox.label() } else { plain };
            if worktrees {
                v.push(it(format!("Open {tool} in a new worktree ({how})"), true, sandboxed));
            }
            v.push(it(format!("Open {tool} {here} ({how})"), false, sandboxed));
        }
        v
    }

    /// Opens a terminal session (FF-23). In a new worktree it is a run of
    /// its own (listed, committed when the window closes); elsewhere the
    /// window is on its own.
    pub(super) fn agent_terminal(
        &mut self,
        path: Option<PathBuf>,
        worktree: bool,
        sandboxed: bool,
        preset: Option<&'static str>,
    ) {
        let mode = ffwd_agent::run::Mode::Interactive { sandboxed };
        let (cfg, _) = self.agent_config_for(preset);
        if worktree {
            let title = self.agent_view.as_ref().map(|v| v.input.text().trim().to_string()).unwrap_or_default();
            return self.launch_agent_with(cfg, title, mode);
        }
        let Some(dir) = path.or_else(|| self.tab_path()) else { return };
        let sink = (self.sinks)(0);
        self.set_notice("Opening a terminal session…".into());
        std::thread::spawn(move || sink(Msg::AgentNotice(ffwd_agent::run::open_in_repo(&cfg, &dir, sandboxed))));
    }

    pub(super) fn on_agent_notice(&mut self, r: Result<String, String>) {
        let (ok, m) = match r {
            Ok(m) => (true, m),
            Err(e) => (false, format!("The terminal session did not open: {e}")),
        };
        self.agent_msg(ok, m);
    }

    fn launch_agent_with(&mut self, cfg: AgentConfig, prompt: String, mode: ffwd_agent::run::Mode) {
        let refusal = match mode {
            ffwd_agent::run::Mode::Prompt => cfg.refusal(),
            ffwd_agent::run::Mode::Interactive { sandboxed } => cfg.refusal_interactive(sandboxed),
        };
        if let Some(why) = refusal {
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
        let title_src = if prompt.trim().is_empty() { "terminal session".to_string() } else { prompt.clone() };
        let (id, handle) = ffwd_agent::run::start_mode(cfg, tab.clone(), prompt.clone(), mode, self.agent_emit());
        self.agents.push(AgentRun {
            tab: tab.clone(),
            rec: Record {
                id: id.clone(),
                repo: tab,
                worktree: PathBuf::new(),
                branch: format!("{prefix}{id}"),
                base: String::new(),
                title: ffwd_agent::run::title(&title_src),
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
                if was_active != rec_active_now(&self.agents[i].rec) || now_done {
                    self.rebuild_side();
                    self.damage.rects.push(self.lay.side);
                }
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
                return Some(ACCENT());
            }
            if !a.seen {
                out = Some(if a.rec.status == Status::Done { ADD() } else { DEL() });
            }
        }
        out
    }

    fn selected_run(&self) -> Option<usize> {
        let v = self.agent_view.as_ref()?;
        self.ordered_runs().get(v.sel).copied()
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
            Btn::Review => {
                let tip = self
                    .r
                    .refs
                    .refs
                    .iter()
                    .find(|r| r.kind == RefKind::Local && *r.name == *rec.branch)
                    .map(|r| r.target);
                match (tip, ffwd_core::Oid::from_hex(rec.base.as_bytes())) {
                    (Some(tip), Some(base)) => {
                        self.close_agents();
                        self.leave_changes();
                        self.leave_files();
                        self.leave_activity();
                        self.leave_reflog();
                        self.side_select_ref(&rec.branch, RefKind::Local);
                        if let Some(&row) = self.r.by_id.get(&tip) {
                            self.select(row as usize);
                        }
                        self.start_compare(base, tip);
                        self.set_notice(format!("Changes of {} against where it started", rec.branch));
                    }
                    _ => {
                        self.refresh_repo(&tab);
                        self.agent_msg(
                            false,
                            format!("Loading {} into the history; try again in a moment.", rec.branch),
                        );
                    }
                }
            }
            Btn::Terminal => {
                let tool = self.agent_view.as_ref().and_then(|v| v.tool);
                self.agent_terminal(Some(rec.worktree.clone()), false, true, tool);
            }
            Btn::Again => {
                if let Some(v) = &mut self.agent_view {
                    v.input = TextField::new(&rec.title);
                    v.input.select_all();
                    v.field = true;
                }
            }
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
        let n = self.ordered_runs().len();
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
            Key::Enter => return self.agent_button(Btn::Review),
            Key::Char('r') => return self.agent_button(Btn::Review),
            Key::Char('t') => return self.agent_button(Btn::Terminal),
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
        if let Some(v) = &mut self.agent_view {
            v.focused = true;
        }
        let hit = self.agent_view.as_ref().and_then(|v| v.hits.iter().find(|h| h.0.contains(x, y)).copied());
        let Some((rect, hit)) = hit else { return self.agents_dirty() };
        match hit {
            Hit::Field => {
                if let Some(v) = &mut self.agent_view {
                    v.field = true;
                }
            }
            Hit::Start => return self.start_agent(),
            Hit::Tool => return self.agent_tool_menu(rect),
            Hit::Run(i) => {
                if let Some(v) = &mut self.agent_view {
                    v.field = false;
                    v.sel = i;
                    v.back = 0;
                    v.ask = None;
                }
            }
            Hit::Terminal => {
                let items = self.agent_terminal_menu(None, true);
                self.open_menu(rect.x, rect.bottom() + self.px(4.0), items);
                return;
            }
            Hit::Button(b) => self.agent_button(b),
            Hit::Confirm(yes) => self.agent_confirm(yes),
            Hit::Tab(t) => {
                if let Some(v) = &mut self.agent_view {
                    v.tab = t;
                }
            }
            Hit::Settings => {
                self.close_agents();
                return self.open_settings_page(1);
            }
            Hit::Close => return self.close_agents(),
            Hit::TurnOn => {
                self.settings.set_string("agents.enabled", "true");
                self.agents_loaded.clear();
                self.load_agent_runs();
            }
            Hit::Example(k) => {
                if let Some(v) = &mut self.agent_view {
                    v.input = TextField::new(EXAMPLES[k.min(EXAMPLES.len() - 1)]);
                    v.input.select_all();
                    v.field = true;
                }
            }
        }
        self.agents_dirty();
    }

    pub(super) fn agents_wheel(&mut self, lines: i64) {
        let over_rail = self.pointer_at.is_some_and(|(x, y)| self.agents_rail().contains(x, y));
        let step = self.px(40.0);
        if let Some(v) = &mut self.agent_view {
            if over_rail {
                v.rail_scroll = (v.rail_scroll + lines as i32 * step).max(0);
            } else {
                v.back = (v.back as i64 - lines).max(0) as usize;
            }
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

    pub(super) fn paint_agents(&mut self, cv: &mut Canvas) {
        let Some(mut view) = self.agent_view.take() else { return };
        let r = self.agents_rect();
        let old = cv.set_clip(cv.clip().intersect(r));
        let (ui, semi, mono) = (self.f.ui, self.f.semi, self.f.mono);
        let mut hits: Vec<(Rect, Hit)> = Vec::new();
        let (cfg, cfg_errors) = self.agent_config_for(view.tool);
        cv.fill(r, BG());

        // Header: title, what runs where (a chip to the settings), Done.
        let head = Rect::new(r.x, r.y, r.w, self.px(64.0));
        cv.fill(Rect::new(head.x, head.bottom() - 1, head.w, 1), BORDER_SOFT());
        let hb = head.y as f32 + self.pf(38.0);
        let e = self.te.draw(cv, semi, self.pf(18.0), r.x as f32 + self.pf(24.0), hb, "Agents", TEXT(), 200.0);
        let e = self.pill(cv, e + self.pf(10.0), hb - self.pf(6.0), "EXPERIMENTAL", TAG_BG(), TAG_FG());
        let repo = self.r.repo_name.clone();
        self.te.draw(cv, ui, self.pf(13.0), e + self.pf(12.0), hb, &repo, TEXT_3(), self.pf(300.0));
        let done_w = (self.te.measure(semi, self.pf(12.5), "Done") + self.pf(28.0)) as i32;
        let done = Rect::new(head.right() - self.px(24.0) - done_w, head.y + self.px(14.0), done_w, self.px(36.0));
        let (dbg, dborder) = self.control_colors(Hot::Agent(hits.len()), ACCENT_SOFT_HOVER(), ACCENT_BORDER());
        cv.round_border(done, self.pf(8.0), dborder, dbg);
        let db = (done.y as f32 + done.h as f32 / 2.0 + self.pf(4.5)).round();
        self.te.draw(cv, semi, self.pf(12.5), done.x as f32 + self.pf(14.0), db, "Done", ACCENT_FG(), done_w as f32);
        hits.push((done, Hit::Close));
        // The chip: tool · sandbox · network, or why it cannot run.
        let tool_name = cfg.preset.map_or_else(|| "Custom".to_string(), |p| p.name.to_string());
        let refusal = cfg.refusal();
        let (chip, chip_fg, dot) = match (&refusal, cfg.sandbox.resolve()) {
            _ if !cfg.enabled => ("Agents are off".to_string(), TEXT_3(), TEXT_4()),
            (Some(why), _) => (why.clone(), MOD(), MOD()),
            (None, ffwd_agent::SandboxKind::None) => {
                (format!("{tool_name} · no sandbox: all your permissions"), DEL_TEXT(), DEL())
            }
            (None, _) => (
                format!("{tool_name} · {} · network {}", cfg.sandbox.label(), if cfg.network { "on" } else { "off" }),
                TEXT_2(),
                ADD(),
            ),
        };
        let chip = match cfg_errors.first() {
            Some(err) => format!("{chip} · settings: {err}"),
            None => chip,
        };
        let max_chip = (done.x - self.px(12.0) - (e as i32 + self.px(60.0))).max(self.px(120.0));
        let cw = ((self.te.measure(ui, self.pf(12.5), &chip) + self.pf(60.0)) as i32).min(max_chip);
        let cr = Rect::new(done.x - self.px(12.0) - cw, done.y, cw, done.h);
        let (cbg, cborder) = self.control_colors(Hot::Agent(hits.len()), BG_INPUT(), BORDER());
        cv.round_border(cr, self.pf(8.0), cborder, cbg);
        cv.fill_circle(cr.x as f32 + self.pf(16.0), cr.y as f32 + cr.h as f32 / 2.0, self.pf(4.0), dot);
        self.te.draw(cv, ui, self.pf(12.5), cr.x as f32 + self.pf(28.0), db, &chip, chip_fg, cw as f32 - self.pf(44.0));
        self.glyph(
            cv,
            Glyph::Gear,
            cr.right() as f32 - self.pf(16.0),
            cr.y as f32 + cr.h as f32 / 2.0,
            self.pf(6.0),
            TEXT_3(),
        );
        hits.push((cr, Hit::Settings));

        // Rail: the composer, then the runs by group.
        let rail = self.agents_rail();
        cv.fill(rail, BG_SIDEBAR());
        cv.fill(Rect::new(rail.right() - 1, rail.y, 1, rail.h), BORDER_SOFT());
        let rx = rail.x + self.px(16.0);
        let rw = rail.w - self.px(32.0);
        let comp = Rect::new(rx, rail.y + self.px(16.0), rw, self.px(110.0));
        cv.round_border(comp, self.pf(10.0), if view.field { ACCENT_BORDER() } else { BORDER() }, BG_PANEL());
        let fr = Rect::new(comp.x + self.px(12.0), comp.y + self.px(12.0), comp.w - self.px(24.0), self.px(42.0));
        cv.round_border(fr, self.pf(8.0), if view.field { ACCENT() } else { BORDER() }, BG_INPUT());
        let fx = fr.x as f32 + self.pf(12.0);
        let fb = (fr.y as f32 + fr.h as f32 / 2.0 + self.pf(4.5)).round();
        let fmax = fr.w as f32 - self.pf(24.0);
        if view.input.text().is_empty() {
            self.te.draw(cv, ui, self.pf(13.0), fx, fb, "What should the agent do?", TEXT_4(), fmax);
        } else {
            if view.input.all_selected() {
                let w = self.te.measure(ui, self.pf(13.0), view.input.text()).min(fmax);
                cv.fill_round(
                    Rect::new(fx as i32 - 2, fr.y + self.px(9.0), w as i32 + 4, fr.h - self.px(18.0)),
                    self.pf(3.0),
                    BG_TEXT_SELECT(),
                );
            }
            // Long tasks show their end while typing.
            let text = view.input.text().to_string();
            let tw = self.te.measure(ui, self.pf(13.0), &text);
            let shift = if view.field && tw > fmax { tw - fmax } else { 0.0 };
            let oc = cv.set_clip(cv.clip().intersect(fr.inset(self.px(8.0), 0)));
            self.te.draw(cv, ui, self.pf(13.0), fx - shift, fb, &text, TEXT(), fmax + shift);
            if view.field {
                let cx = fx - shift + self.te.measure(ui, self.pf(13.0), &text[..view.input.cursor()]);
                cv.fill(
                    Rect::new(cx as i32 + 1, fr.y + self.px(10.0), self.px(1.5).max(1), fr.h - self.px(20.0)),
                    ACCENT(),
                );
            }
            cv.set_clip(oc);
        }
        if view.field && view.input.text().is_empty() {
            cv.fill(
                Rect::new(fx as i32 + 1, fr.y + self.px(10.0), self.px(1.5).max(1), fr.h - self.px(20.0)),
                ACCENT(),
            );
        }
        hits.push((fr, Hit::Field));
        // Tool, terminal, start.
        let by = fr.bottom() + self.px(12.0);
        let bh = self.px(32.0);
        let tl = view.tool.and_then(ffwd_agent::presets::find).map_or(tool_name.clone(), |p| p.name.to_string());
        let tw = ((self.te.measure(semi, self.pf(12.5), &tl) + self.pf(52.0)) as i32).min(comp.w - self.px(220.0));
        let tr = Rect::new(fr.x, by, tw, bh);
        let (tbg, tborder) = self.control_colors(Hot::Agent(hits.len()), BG_INPUT(), BORDER());
        cv.round_border(tr, self.pf(7.0), tborder, tbg);
        let tb = (tr.y as f32 + tr.h as f32 / 2.0 + self.pf(4.5)).round();
        self.glyph(
            cv,
            Glyph::Terminal,
            tr.x as f32 + self.pf(14.0),
            tr.y as f32 + tr.h as f32 / 2.0,
            self.pf(5.5),
            TEXT_3(),
        );
        self.te.draw(cv, semi, self.pf(12.5), tr.x as f32 + self.pf(28.0), tb, &tl, TEXT(), tw as f32 - self.pf(44.0));
        self.chevron_small(cv, tr.right() as f32 - self.pf(13.0), tr.y as f32 + tr.h as f32 / 2.0, TEXT_3());
        hits.push((tr, Hit::Tool));
        let can = !view.input.text().trim().is_empty() && refusal.is_none();
        let sl = "Start";
        let sw = (self.te.measure(semi, self.pf(12.5), sl) + self.pf(40.0)) as i32;
        let sr = Rect::new(fr.right() - sw, by, sw, bh);
        let hot = self.is_hot(Hot::Agent(hits.len()));
        let (sbg, sfg) = match (can, hot) {
            (false, _) => (BG_HOVER(), TEXT_4()),
            (true, true) => (ACCENT_TEXT(), ACCENT()),
            (true, false) => (ACCENT(), ACCENT_TEXT()),
        };
        cv.fill_round(sr, self.pf(7.0), if can && hot { ACCENT_FG() } else { sbg });
        let _ = sfg;
        self.te.draw(
            cv,
            semi,
            self.pf(12.5),
            sr.x as f32 + self.pf(14.0),
            tb,
            sl,
            if can { ACCENT_TEXT() } else { TEXT_4() },
            sw as f32,
        );
        self.play(
            cv,
            sr.right() as f32 - self.pf(15.0),
            sr.y as f32 + sr.h as f32 / 2.0,
            if can { ACCENT_TEXT() } else { TEXT_4() },
        );
        hits.push((sr, Hit::Start));
        let ol = "Terminal";
        let ow = (self.te.measure(semi, self.pf(12.5), ol) + self.pf(40.0)) as i32;
        let orr = Rect::new(sr.x - self.px(8.0) - ow, by, ow, bh);
        let (obg, oborder) = self.control_colors(Hot::Agent(hits.len()), BG_INPUT(), BORDER());
        cv.round_border(orr, self.pf(7.0), oborder, obg);
        self.glyph(
            cv,
            Glyph::Terminal,
            orr.x as f32 + self.pf(15.0),
            orr.y as f32 + bh as f32 / 2.0,
            self.pf(5.5),
            TEXT_2(),
        );
        self.te.draw(cv, semi, self.pf(12.5), orr.x as f32 + self.pf(28.0), tb, ol, TEXT(), ow as f32);
        hits.push((orr, Hit::Terminal));
        let mut y = comp.bottom() + self.px(8.0);
        if let Some((ok, m)) = view.msg.clone() {
            for line in self.wrap_px(ui, self.pf(12.0), &m, rw as f32).iter().take(3) {
                y += self.px(17.0);
                self.te.draw(
                    cv,
                    ui,
                    self.pf(12.0),
                    rx as f32,
                    y as f32,
                    line,
                    if ok { TEXT_2() } else { MOD() },
                    rw as f32,
                );
            }
            y += self.px(8.0);
        }
        // The run cards.
        let runs = self.ordered_runs();
        view.sel = view.sel.min(runs.len().saturating_sub(1));
        let list = Rect::new(rail.x, y + self.px(6.0), rail.w, rail.bottom() - y - self.px(6.0));
        let oc = cv.set_clip(cv.clip().intersect(list));
        let card_h = self.px(74.0);
        let mut cy = list.y + self.px(6.0) - view.rail_scroll;
        let mut last = None;
        let now = now_secs();
        for (k, &i) in runs.iter().enumerate() {
            let g = group_of(&self.agents[i].rec);
            if last != Some(g) {
                cy += self.px(if last.is_some() { 14.0 } else { 4.0 });
                let n = runs.iter().filter(|&&j| group_of(&self.agents[j].rec) == g).count();
                let t = format!("{}  {n}", GROUPS[g].to_uppercase());
                self.te.draw(
                    cv,
                    semi,
                    self.pf(10.5),
                    rx as f32 + self.pf(4.0),
                    cy as f32 + self.pf(12.0),
                    &t,
                    TEXT_4(),
                    rw as f32,
                );
                cy += self.px(22.0);
                last = Some(g);
            }
            let cr = Rect::new(rx, cy, rw, card_h - self.px(6.0));
            cy += card_h;
            if cr.bottom() < list.y || cr.y > list.bottom() {
                continue;
            }
            let sel = k == view.sel;
            let hot = self.is_hot(Hot::Agent(hits.len()));
            let bg = if sel {
                BG_SELECT_DIM()
            } else if hot {
                BG_HOVER()
            } else {
                BG_PANEL()
            };
            cv.round_border(cr, self.pf(9.0), if sel { ACCENT_BORDER() } else { BORDER_SOFT() }, bg);
            let a = &self.agents[i];
            let rec = a.rec.clone();
            let phase = a.phase.clone();
            let last_line = a.log.iter().rev().find(|l| !l.trim().is_empty() && !l.starts_with("» ")).cloned();
            let fresh = !a.seen;
            let color = status_color(rec.status);
            let tx = cr.x as f32 + self.pf(28.0);
            let tw = cr.w as f32 - self.pf(40.0);
            cv.fill_circle(cr.x as f32 + self.pf(15.0), cr.y as f32 + self.pf(20.0), self.pf(4.5), color);
            if fresh {
                cv.fill_circle(cr.right() as f32 - self.pf(14.0), cr.y as f32 + self.pf(16.0), self.pf(3.5), ACCENT());
            }
            self.te.draw(
                cv,
                semi,
                self.pf(13.0),
                tx,
                cr.y as f32 + self.pf(24.0),
                &rec.title,
                TEXT(),
                tw - self.pf(10.0),
            );
            let age = fmt::relative(rec.started, now, self.r.tz);
            let state = if rec.status.active() && !phase.is_empty() {
                format!("{} · {}", phase.to_lowercase(), age)
            } else {
                format!("{} · {}", rec.status.word(), age)
            };
            self.te.draw(cv, semi, self.pf(11.5), tx, cr.y as f32 + self.pf(43.0), &state, color, tw);
            let third = match (rec.status.active(), last_line) {
                (true, Some(l)) => l,
                _ if !rec.shortstat.is_empty() => rec.shortstat.clone(),
                _ if !rec.detail.is_empty() => rec.detail.clone(),
                _ => rec.branch.clone(),
            };
            self.te.draw(cv, mono, self.pf(11.0), tx, cr.y as f32 + self.pf(60.0), &third, TEXT_3(), tw);
            hits.push((cr, Hit::Run(k)));
        }
        let content_end = cy + view.rail_scroll;
        view.rail_scroll = view.rail_scroll.min((content_end - list.bottom()).max(0));
        cv.set_clip(oc);

        // The right side: the selected run, or how to start.
        let dr = Rect::new(rail.right(), rail.y, r.right() - rail.right(), rail.h);
        let pad = self.px(32.0);
        let dx = dr.x + pad;
        let dw = (dr.w - 2 * pad).min(self.px(980.0));
        if !cfg.enabled {
            self.agents_empty(cv, dx, dr.y + pad, dw, true, &mut hits);
        } else if runs.is_empty() {
            self.agents_empty(cv, dx, dr.y + pad, dw, false, &mut hits);
        } else if let Some(&i) = runs.get(view.sel) {
            let a = &self.agents[i];
            let rec = a.rec.clone();
            let has_handle = a.handle.is_some();
            let active = rec.status.active();
            let has_tree = !rec.worktree.as_os_str().is_empty();
            let mut yy = dr.y as f32 + self.pf(44.0);
            for line in self.wrap_px(semi, self.pf(19.0), &rec.title, dw as f32).iter().take(2) {
                self.te.draw(cv, semi, self.pf(19.0), dx as f32, yy, line, TEXT(), dw as f32);
                yy += self.pf(26.0);
            }
            // Status pill, branch, base, when, sandbox.
            let color = status_color(rec.status);
            let word = rec.status.word().to_uppercase();
            let e = self.pill(cv, dx as f32, yy - self.pf(4.0), &word, BG_HOVER(), color);
            let base = if rec.base.is_empty() {
                String::new()
            } else {
                format!("  from {}", &rec.base[..rec.base.len().min(8)])
            };
            let e = self.te.draw(
                cv,
                mono,
                self.pf(12.0),
                e + self.pf(10.0),
                yy,
                &format!("{}{base}", rec.branch),
                REN(),
                dw as f32,
            );
            let when = fmt::relative(rec.started, now_secs(), self.r.tz);
            self.te.draw(
                cv,
                ui,
                self.pf(12.0),
                e + self.pf(10.0),
                yy,
                &format!("· {when} · {}", rec.sandbox),
                TEXT_3(),
                dx as f32 + dw as f32 - e,
            );
            yy += self.pf(16.0);
            // What happened, in one line.
            let result = match (&rec.detail, rec.status) {
                (d, _) if !d.is_empty() => d.clone(),
                (_, s) if s.active() => {
                    "Working. The output below is live; the branch fills as the agent commits.".into()
                }
                _ => String::new(),
            };
            if !result.is_empty() {
                yy += self.pf(10.0);
                for line in self.wrap_px(ui, self.pf(12.5), &result, dw as f32).iter().take(2) {
                    self.te.draw(cv, ui, self.pf(12.5), dx as f32, yy + self.pf(4.0), line, TEXT_2(), dw as f32);
                    yy += self.pf(18.0);
                }
            }
            yy += self.pf(16.0);
            // Actions that fit the state; the first is the main one.
            let to_review = rec.status == Status::Done && rec.commits > 0;
            let buttons: Vec<(Btn, &str, bool)> = if active {
                vec![
                    (Btn::Cancel, "Cancel run", has_handle),
                    (Btn::Folder, "Open folder", has_tree),
                    (Btn::Log, "Open log", has_tree),
                ]
            } else if to_review {
                vec![
                    (Btn::Review, "Review changes", true),
                    (Btn::Merge, "Merge…", true),
                    (Btn::Terminal, "Continue in terminal", has_tree),
                    (Btn::Again, "Run again", true),
                    (Btn::Discard, "Discard…", true),
                ]
            } else if rec.status == Status::Merged {
                vec![
                    (Btn::Show, "Show branch", true),
                    (Btn::Again, "Run again", true),
                    (Btn::Discard, "Discard…", true),
                ]
            } else {
                vec![
                    (Btn::Again, "Run again", true),
                    (Btn::Log, "Open log", has_tree),
                    (Btn::Terminal, "Open terminal here", has_tree),
                    (Btn::Discard, "Discard…", has_tree),
                ]
            };
            let mut bx = dx;
            let by = yy as i32;
            for (k, (b, label, en)) in buttons.into_iter().enumerate() {
                let primary = k == 0 && en && !matches!(b, Btn::Cancel);
                let danger = matches!(b, Btn::Discard | Btn::Cancel);
                let br = self.agent_btn2(cv, bx, by, label, en, primary, danger, hits.len());
                if en {
                    hits.push((br, Hit::Button(b)));
                }
                bx = br.right() + self.px(8.0);
            }
            let mut ly = by + self.px(48.0);
            // Confirmation.
            if let Some(ask) = view.ask {
                let text = self.ask_text(ask, &rec);
                let (sz, maxw) = (self.pf(12.5), dw as f32 - self.pf(28.0));
                let lines = self.wrap_px(ui, sz, &text, maxw);
                let h = self.px(20.0) + lines.len() as i32 * self.px(18.0) + self.px(48.0);
                let cr = Rect::new(dx, ly, dw, h);
                let danger = ask == Ask::Discard;
                cv.round_border(
                    cr,
                    self.pf(9.0),
                    if danger { DANGER_BORDER() } else { HUNK_BORDER() },
                    if danger { DEL_BG() } else { HUNK_BG() },
                );
                for (k, l) in lines.iter().enumerate() {
                    self.te.draw(
                        cv,
                        ui,
                        self.pf(12.5),
                        cr.x as f32 + self.pf(14.0),
                        cr.y as f32 + self.pf(24.0) + k as f32 * self.pf(18.0),
                        l,
                        TEXT(),
                        cr.w as f32,
                    );
                }
                let byy = cr.bottom() - self.px(42.0);
                let yes = if danger { "Discard (Enter)" } else { "Merge (Enter)" };
                let b1 = self.agent_btn2(cv, cr.x + self.px(14.0), byy, yes, true, !danger, danger, hits.len());
                hits.push((b1, Hit::Confirm(true)));
                let b2 = self.agent_btn2(
                    cv,
                    b1.right() + self.px(8.0),
                    byy,
                    "Keep it (Esc)",
                    true,
                    false,
                    false,
                    hits.len(),
                );
                hits.push((b2, Hit::Confirm(false)));
                ly = cr.bottom() + self.px(14.0);
            }
            // Output and Details.
            let mut tx = dx;
            for (t, name) in [(0u8, "Output"), (1u8, "Details")] {
                let on = view.tab == t;
                let w = (self.te.measure(semi, self.pf(12.5), name) + self.pf(24.0)) as i32;
                let trr = Rect::new(tx, ly, w, self.px(30.0));
                if on {
                    cv.fill_round(trr, self.pf(7.0), BG_CHOSEN());
                } else if self.is_hot(Hot::Agent(hits.len())) {
                    cv.fill_round(trr, self.pf(7.0), BG_HOVER());
                }
                self.te.draw(
                    cv,
                    semi,
                    self.pf(12.5),
                    tx as f32 + self.pf(12.0),
                    ly as f32 + self.pf(20.0),
                    name,
                    if on { TEXT() } else { TEXT_3() },
                    w as f32,
                );
                hits.push((trr, Hit::Tab(t)));
                tx = trr.right() + self.px(4.0);
            }
            if active {
                cv.fill_circle(tx as f32 + self.pf(12.0), ly as f32 + self.pf(15.0), self.pf(3.5), ACCENT());
                self.te.draw(
                    cv,
                    ui,
                    self.pf(11.5),
                    tx as f32 + self.pf(20.0),
                    ly as f32 + self.pf(19.5),
                    "live",
                    ACCENT_FG(),
                    60.0,
                );
            }
            let br = Rect::new(dx, ly + self.px(38.0), dw, dr.bottom() - pad - (ly + self.px(38.0)));
            if br.h > self.px(30.0) {
                cv.round_border(br, self.pf(9.0), BORDER_SOFT(), BG_SIDEBAR());
                if view.tab == 0 {
                    self.agents_log(cv, br, i, active, &mut view);
                } else {
                    self.agents_details(cv, br, &rec);
                }
            }
        }
        cv.set_clip(old);
        view.hits = hits;
        self.agent_view = Some(view);
    }

    /// The log of run `i` in `lr`, newest at the bottom.
    fn agents_log(&mut self, cv: &mut Canvas, lr: Rect, i: usize, active: bool, view: &mut AgentView) {
        let (ui, mono) = (self.f.ui, self.f.mono);
        let lh = self.px(18.0);
        let fit = ((lr.h - self.px(20.0)) / lh).max(0) as usize;
        let log = &self.agents[i].log;
        if log.is_empty() {
            let t = if active {
                "Waiting for output…"
            } else {
                "No output in this session. Open log shows the saved log."
            };
            self.te.draw(
                cv,
                ui,
                self.pf(12.5),
                lr.x as f32 + self.pf(14.0),
                lr.y as f32 + self.pf(26.0),
                t,
                TEXT_4(),
                lr.w as f32,
            );
            return;
        }
        let total = log.len();
        view.back = view.back.min(total.saturating_sub(fit));
        let end = total - view.back;
        let start = end.saturating_sub(fit);
        let lines: Vec<String> = log.range(start..end).cloned().collect();
        for (k, l) in lines.iter().enumerate() {
            let step = l.starts_with("» ");
            let c = if step { HUNK_TEXT() } else { TEXT_2() };
            let y = lr.y as f32 + self.pf(22.0) + (k as i32 * lh) as f32;
            self.te.draw(cv, mono, self.pf(11.5), lr.x as f32 + self.pf(14.0), y, l, c, lr.w as f32 - self.pf(28.0));
        }
        if view.back > 0 {
            let t = format!("↓ {} newer lines", view.back);
            let w = self.te.measure(ui, self.pf(11.0), &t);
            self.te.draw(
                cv,
                ui,
                self.pf(11.0),
                lr.right() as f32 - w - self.pf(14.0),
                lr.y as f32 + self.pf(20.0),
                &t,
                TEXT_3(),
                w + 2.0,
            );
        }
    }

    /// What a run is: task, branch, base, worktree, sandbox, times, result.
    fn agents_details(&mut self, cv: &mut Canvas, br: Rect, rec: &Record) {
        let (ui, semi, mono) = (self.f.ui, self.f.semi, self.f.mono);
        let tz = self.r.tz;
        let when = |t: i64| fmt::relative(t, now_secs(), tz);
        let rows: Vec<(&str, String, bool)> = vec![
            ("Task", rec.title.clone(), false),
            ("Branch", rec.branch.clone(), true),
            ("Started from", rec.base.chars().take(12).collect(), true),
            ("Worktree", rec.worktree.to_string_lossy().into_owned(), true),
            ("Sandbox", rec.sandbox.clone(), false),
            ("Started", when(rec.started), false),
            ("Ended", rec.ended.map_or("—".into(), when), false),
            ("Commits", rec.commits.to_string(), false),
            ("Changes", if rec.shortstat.is_empty() { "—".into() } else { rec.shortstat.clone() }, false),
            ("Result", if rec.detail.is_empty() { rec.status.word().to_string() } else { rec.detail.clone() }, false),
        ];
        let kx = br.x as f32 + self.pf(16.0);
        let vx = br.x as f32 + self.pf(140.0);
        let vw = br.right() as f32 - vx - self.pf(16.0);
        let mut y = br.y as f32 + self.pf(28.0);
        for (k, v, code) in rows {
            if y > br.bottom() as f32 - self.pf(8.0) {
                break;
            }
            self.te.draw(cv, semi, self.pf(12.0), kx, y, k, TEXT_3(), self.pf(120.0));
            self.te.draw(cv, if code { mono } else { ui }, self.pf(12.5), vx, y, &v, TEXT(), vw);
            y += self.pf(24.0);
        }
    }

    /// The right side with no run to show: agents off (Turn on), or how to
    /// start, with example tasks.
    fn agents_empty(&mut self, cv: &mut Canvas, x: i32, y: i32, w: i32, off: bool, hits: &mut Vec<(Rect, Hit)>) {
        let (ui, semi) = (self.f.ui, self.f.semi);
        let w = w.min(self.px(720.0));
        let mut yy = y as f32 + self.pf(30.0);
        let (title, text) = if off {
            (
                "Agents are off",
                "Turn them on to give coding tasks to an agent. Each task runs in its own worktree and branch, inside a sandbox; your checkout is never touched, and nothing is merged until you say so.",
            )
        } else {
            (
                "Give an agent a task",
                "Describe what you want in the field on the left and press Start. The agent works in its own worktree and branch, inside a sandbox. When it is done you review its changes, then merge or discard them.",
            )
        };
        self.te.draw(cv, semi, self.pf(22.0), x as f32, yy, title, TEXT(), w as f32);
        yy += self.pf(14.0);
        for line in self.wrap_px(ui, self.pf(13.5), text, w as f32) {
            yy += self.pf(21.0);
            self.te.draw(cv, ui, self.pf(13.5), x as f32, yy, &line, TEXT_2(), w as f32);
        }
        yy += self.pf(26.0);
        if off {
            let mut bx = x;
            for (label, hit, primary) in [("Turn on", Hit::TurnOn, true), ("Agent settings", Hit::Settings, false)] {
                let br = self.agent_btn2(cv, bx, yy as i32, label, true, primary, false, hits.len());
                hits.push((br, hit));
                bx = br.right() + self.px(8.0);
            }
            return;
        }
        self.te.draw(cv, semi, self.pf(10.5), x as f32, yy, "TRY", TEXT_4(), 100.0);
        yy += self.pf(10.0);
        for (k, ex) in EXAMPLES.iter().enumerate() {
            let er = Rect::new(x, yy as i32, w, self.px(40.0));
            let (bg, border) = self.control_colors(Hot::Agent(hits.len()), BG_PANEL(), BORDER_SOFT());
            cv.round_border(er, self.pf(9.0), border, bg);
            self.glyph(
                cv,
                Glyph::Plus,
                er.x as f32 + self.pf(18.0),
                er.y as f32 + er.h as f32 / 2.0,
                self.pf(5.0),
                ACCENT(),
            );
            self.te.draw(
                cv,
                ui,
                self.pf(13.0),
                er.x as f32 + self.pf(34.0),
                er.y as f32 + self.pf(25.0),
                ex,
                TEXT(),
                er.w as f32 - self.pf(48.0),
            );
            hits.push((er, Hit::Example(k)));
            yy += self.pf(48.0);
        }
        yy += self.pf(20.0);
        self.te.draw(cv, semi, self.pf(10.5), x as f32, yy, "HOW IT WORKS", TEXT_4(), 200.0);
        let steps = [
            ("1", "Its own branch", "A new branch and worktree; your checkout stays as it is."),
            ("2", "In a sandbox", "It sees its worktree and only the paths you allow."),
            ("3", "You decide", "Review changes opens the diff; Merge asks first."),
        ];
        let col = (w - self.px(24.0)) / 3;
        for (k, (n, head, sub)) in steps.iter().enumerate() {
            let sx = x + k as i32 * (col + self.px(12.0));
            let sr = Rect::new(sx, yy as i32 + self.px(12.0), col, self.px(96.0));
            cv.round_border(sr, self.pf(9.0), BORDER_SOFT(), BG_SIDEBAR());
            cv.fill_circle(sr.x as f32 + self.pf(22.0), sr.y as f32 + self.pf(24.0), self.pf(11.0), ACCENT_SOFT());
            self.te.draw(
                cv,
                semi,
                self.pf(12.0),
                sr.x as f32 + self.pf(18.5),
                sr.y as f32 + self.pf(28.5),
                n,
                ACCENT_FG(),
                20.0,
            );
            self.te.draw(
                cv,
                semi,
                self.pf(12.5),
                sr.x as f32 + self.pf(42.0),
                sr.y as f32 + self.pf(28.5),
                head,
                TEXT(),
                sr.w as f32 - self.pf(50.0),
            );
            let mut sy = sr.y as f32 + self.pf(50.0);
            for line in self.wrap_px(ui, self.pf(12.0), sub, sr.w as f32 - self.pf(28.0)).iter().take(2) {
                self.te.draw(cv, ui, self.pf(12.0), sr.x as f32 + self.pf(14.0), sy, line, TEXT_3(), sr.w as f32);
                sy += self.pf(17.0);
            }
        }
    }

    /// A page button: primary (accent), danger (red) or plain; `hit` is
    /// its index in the hit list being built.
    #[allow(clippy::too_many_arguments)]
    fn agent_btn2(
        &mut self,
        cv: &mut Canvas,
        x: i32,
        y: i32,
        label: &str,
        enabled: bool,
        primary: bool,
        danger: bool,
        hit: usize,
    ) -> Rect {
        let semi = self.f.semi;
        let w = (self.te.measure(semi, self.pf(12.5), label) + self.pf(28.0)) as i32;
        let r = Rect::new(x, y, w, self.px(34.0));
        let h = Hot::Agent(hit);
        let (border, bg, fg) = match (enabled, primary, danger) {
            (false, _, _) => (BORDER_SOFT(), BG_PANEL(), TEXT_4()),
            (true, true, _) if self.is_hot(h) => (ACCENT_FG(), ACCENT_FG(), ACCENT_TEXT()),
            (true, true, _) => (ACCENT(), ACCENT(), ACCENT_TEXT()),
            (true, false, true) if self.is_hot(h) => (DEL(), DEL_BG(), DEL_TEXT()),
            (true, false, true) => (DANGER_BORDER(), BG_PANEL(), DEL_TEXT()),
            (true, false, false) => {
                let (bg, border) = self.control_colors(h, BG_BAR(), BORDER());
                (border, bg, TEXT())
            }
        };
        cv.round_border(r, self.pf(8.0), border, bg);
        self.te.draw(cv, semi, self.pf(12.5), x as f32 + self.pf(14.0), y as f32 + self.pf(21.5), label, fg, w as f32);
        r
    }

    /// Word-wraps `text` to `maxw` pixels.
    fn wrap_px(&mut self, f: FontId, size: f32, text: &str, maxw: f32) -> Vec<String> {
        wrap(&mut self.te, f, size, text, maxw)
    }

    fn chevron_small(&self, cv: &mut Canvas, cx: f32, cy: f32, color: u32) {
        let s = self.pf(3.2);
        cv.stroke(&[(cx - s, cy - s / 2.0), (cx, cy + s / 2.0), (cx + s, cy - s / 2.0)], self.pf(1.4), color);
    }

    /// A small "play" triangle (Start).
    fn play(&self, cv: &mut Canvas, cx: f32, cy: f32, color: u32) {
        let s = self.pf(4.0);
        cv.stroke(
            &[(cx - s * 0.6, cy - s), (cx + s * 0.8, cy), (cx - s * 0.6, cy + s), (cx - s * 0.6, cy - s)],
            self.pf(1.6),
            color,
        );
    }

    /// Tests: the page's click targets and what they are.
    #[cfg(test)]
    pub(super) fn agent_view_hits(&self) -> Vec<(Rect, String)> {
        self.agent_view.as_ref().map_or(Vec::new(), |v| v.hits.iter().map(|(r, h)| (*r, format!("{h:?}"))).collect())
    }

    /// Tests: the composer's text.
    #[cfg(test)]
    pub(super) fn agent_input(&self) -> String {
        self.agent_view.as_ref().map_or(String::new(), |v| v.input.text().to_string())
    }

    /// Tests: the rail's run titles, in order.
    #[cfg(test)]
    pub(super) fn agent_rail_titles(&self) -> Vec<String> {
        self.ordered_runs().iter().map(|&i| self.agents[i].rec.title.clone()).collect()
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
        self.open_settings_named("Access");
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
        self.agent_view = Some(AgentView::new("Add keyboard shortcuts to the agents page", false));
        self.damage.full = true;
    }
}

fn rec_active_now(r: &Record) -> bool {
    r.status.active()
}

fn status_color(s: Status) -> u32 {
    match s {
        Status::Preparing | Status::Running | Status::Finishing => ACCENT(),
        Status::Done => ADD(),
        Status::Merged => REN(),
        Status::Cancelled | Status::Interrupted => TEXT_3(),
        Status::Failed | Status::TimedOut => DEL(),
        Status::Discarded => TEXT_4(),
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
            std::process::Command::new("explorer")
                .arg(crate::platform::shell_path(&p))
                .creation_flags(0x0800_0000)
                .spawn()
        };
        #[cfg(target_os = "macos")]
        let _ = std::process::Command::new("open").arg(&p).spawn();
        #[cfg(not(any(windows, target_os = "macos")))]
        let _ = std::process::Command::new("xdg-open").arg(&p).spawn();
    });
}
