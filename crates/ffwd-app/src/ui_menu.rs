//! Context menus (right click, `Ctrl+.`) for commits and refs, and the two
//! small dialogs they open: a name prompt (create branch, create tag,
//! rename) and a choice dialog that says exactly what an operation does
//! before it runs (merge modes, rebase, reset, delete). The operations run
//! on workers (work.rs, ffwd-core refops.rs).

use super::*;
use crate::work::RefOp;
use ffwd_core::refops::{MergeMode, OpError, ResetMode};

#[derive(Clone, Debug)]
pub enum MenuAct {
    Run(RefOp),
    /// Ask first (a choice dialog).
    Ask(Box<Ask>),
    Name(NameAct),
    Copy(String),
    /// Interactive rebase from this commit (FF-37).
    RebaseFrom(String),
    /// Open a submodule (by name) in a new tab.
    OpenSubmodule(String),
    /// Tab actions (FF-44).
    CloseTab(usize),
    CloseOtherTabs(usize),
    OpenFolder(PathBuf),
    OpenTerminal(PathBuf),
    /// Open a web page (the hosting site's page of a commit).
    OpenUrl(String),
    /// Diff view (FF-41): whitespace characters on or off, tab width.
    DiffShowWs(bool),
    DiffTab(u8),
    /// Local Changes: hide (true) or show untracked files.
    HideUntracked(bool),
    /// Local Changes: show (true) or hide ignored files.
    ShowIgnored(bool),
    /// Wrap long diff lines (FF-41).
    DiffWrap(bool),
    /// A diff bar choice from the view menu.
    DiffBar(super::diff_view::DiffBar),
    /// Code text size in points.
    CodeSize(u8),
    /// A tab's colour (FF-44).
    TabColor(usize, Option<u8>),
    /// Sort the sidebar's branches by most recent commit (else by name).
    BranchOrder(bool),
    /// Copy a patch (commit, stash, working changes) to the clipboard.
    CopyPatch(PatchSrc),
    /// `gh pr create --web` for the current branch.
    CreatePr,
    /// Select a commit in the history view.
    ShowCommit(String),
    /// The history's scope (true: the current branch only) and whether
    /// stashes show in it (FF-33).
    Scope(bool),
    ShowStashes(bool),
    /// The agent's interactive CLI in a terminal window (FF-23): in a new
    /// worktree (`worktree`) or in `path` (None: the current tab), in the
    /// sandbox or not.
    /// `preset`: another agent than the configured one (its own command
    /// and grants).
    AgentTerminal {
        path: Option<PathBuf>,
        worktree: bool,
        sandboxed: bool,
        preset: Option<&'static str>,
    },
    /// A settings row's choice from its dropdown, and the agent settings'
    /// scope (true: this repository only).
    SettingChoice(super::settings_view::Row, usize),
    SettingScope(bool),
    /// Open a folder (a worktree) in a new tab.
    OpenTab(PathBuf),
    /// Pick a sidebar item for a batch, or take it out (FF-36).
    SidePick(String),
    /// Pick every item under a sidebar header (`t:`, `b:`, `s:` prefix).
    SidePickAll(&'static str),
    /// Clear the batch selection.
    SidePickClear,
    /// Files (FF-47): open in the editor (or the default app), show in
    /// the folder, external diff and merge tools.
    OpenFile(PathBuf, bool),
    /// Open a file in the editor at a line (FF-41).
    OpenFileAt(PathBuf, u32),
    /// Compare commits (FF-33): mark one, then compare another with it.
    CompareMark(ffwd_core::Oid),
    Compare(ffwd_core::Oid, ffwd_core::Oid),
    /// Local Changes, on the selected file (FF-57): stage or unstage,
    /// discard (asks first), add to .gitignore.
    StageToggle(bool),
    Discard,
    Ignore,
    ShowFile(PathBuf),
    DiffTool(String, ffwd_core::tools::DiffOf),
    MergeTool(String),
}

#[derive(Clone, Debug)]
pub struct MenuItem {
    pub label: String,
    /// `None` is a separator.
    pub act: Option<MenuAct>,
    pub danger: bool,
}

pub struct Menu {
    pub x: i32,
    pub y: i32,
    /// Width: the longest label plus padding and the icon gutter.
    pub w: i32,
    pub items: Vec<MenuItem>,
    pub sel: Option<usize>,
}

#[derive(Clone, Debug)]
pub enum NameAct {
    CreateBranch {
        at: String,
    },
    CreateTag {
        at: String,
    },
    Rename {
        old: String,
    },
    /// Stash the working changes; the toggle includes untracked files.
    StashSave,
    StashRename {
        name: String,
        old: String,
    },
    /// Add a remote: first its URL, then its name.
    AddRemoteUrl,
    AddRemoteName {
        url: String,
    },
    RemoteUrl {
        name: String,
        old: String,
    },
    RenameRemote {
        old: String,
    },
    /// Make `branch` track a remote branch (FF-36); the field starts with
    /// its current upstream or `origin/<branch>`.
    SetUpstream {
        branch: String,
        current: String,
    },
    /// Add a submodule: its URL, then its folder.
    AddSubmoduleUrl,
    AddSubmodulePath {
        url: String,
    },
    /// Move a submodule to another folder.
    MoveSubmodule {
        path: String,
    },
    /// Rename tab `index`.
    RenameTab {
        index: usize,
        old: String,
    },
    /// Clone: the URL, then the folder.
    CloneUrl,
    CloneDest {
        url: String,
    },
    /// A new repository in this folder.
    InitDir,
    /// Save a patch to this file.
    SavePatch {
        src: PatchSrc,
    },
    /// Apply the patch in this file.
    ApplyPatchFile,
}

/// A choice dialog: a title, what will happen, and buttons.
#[derive(Clone, Debug)]
pub struct Ask {
    pub title: String,
    pub text: String,
    /// (label, operation, dangerous); the first is the default.
    pub choices: Vec<(String, RefOp, bool)>,
}

pub enum DialogKind {
    Name { act: NameAct, field: TextField, checkout: bool },
    Ask(Ask),
}

pub struct Dialog {
    pub kind: DialogKind,
    pub sel: usize,
    pub error: Option<String>,
    pub busy: bool,
    pub hits: Vec<(Rect, DHit)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DHit {
    Field,
    Toggle,
    Choice(usize),
    Cancel,
}

fn item(label: impl Into<String>, act: MenuAct) -> MenuItem {
    MenuItem { label: label.into(), act: Some(act), danger: false }
}

fn danger(label: impl Into<String>, act: MenuAct) -> MenuItem {
    MenuItem { label: label.into(), act: Some(act), danger: true }
}

fn sep() -> MenuItem {
    MenuItem { label: String::new(), act: None, danger: false }
}

impl Ui {
    fn head_name(&self) -> Option<String> {
        self.r.refs.head_branch.as_deref().map(str::to_string)
    }

    /// The menu for the commit at `row`.
    pub(super) fn commit_menu(&self, row: usize) -> Vec<MenuItem> {
        let Some(c) = self.r.commits.get(row) else { return Vec::new() };
        // A stash row has the stash's menu (FF-33).
        if let Some(s) = self.r.refs.refs.iter().find(|r| r.kind == RefKind::Stash && r.target == c.id) {
            let mut v = self.ref_menu(&format!("s:{}", s.name));
            v.push(item("Copy SHA", MenuAct::Copy(c.id.to_string())));
            return v;
        }
        let sha = c.id.to_string();
        let short = sha[..sha.len().min(8)].to_string();
        let head = self.head_name();
        let locals: Vec<String> = self
            .r
            .refs
            .refs
            .iter()
            .filter(|r| r.kind == RefKind::Local && r.target == c.id && !r.is_head)
            .map(|r| r.name.to_string())
            .collect();
        let mut v = Vec::new();
        for b in &locals {
            v.push(item(format!("Checkout {b}"), MenuAct::Run(RefOp::Checkout(b.clone()))));
        }
        v.push(item(format!("Checkout {short} (detached)"), MenuAct::Run(RefOp::Checkout(sha.clone()))));
        v.push(sep());
        v.push(item("Create branch here…", MenuAct::Name(NameAct::CreateBranch { at: sha.clone() })));
        v.push(item("Create tag here…", MenuAct::Name(NameAct::CreateTag { at: sha.clone() })));
        if let Some(h) = &head {
            let what = locals.first().cloned().unwrap_or_else(|| short.clone());
            let target = locals.first().cloned().unwrap_or_else(|| sha.clone());
            v.push(sep());
            v.push(item(format!("Merge {what} into {h}…"), MenuAct::Ask(Box::new(self.merge_ask(&target, &what, h)))));
            v.push(item(
                format!("Rebase {h} onto {what}…"),
                MenuAct::Ask(Box::new(self.rebase_ask(&target, &what, h))),
            ));
            v.push(item(format!("Cherry-pick {short} into {h}"), MenuAct::Run(RefOp::CherryPick(sha.clone()))));
            v.push(item(format!("Revert {short}"), MenuAct::Run(RefOp::Revert(sha.clone()))));
            v.push(sep());
            v.push(item(format!("Reset {h} to {short}…"), MenuAct::Ask(Box::new(self.reset_ask(&sha, &short, h)))));
            v.push(item(format!("Interactive rebase {h} from {short}…"), MenuAct::RebaseFrom(sha.clone())));
        }
        v.push(sep());
        // The hosting site's page of this commit (origin, else the first remote).
        let remote = self.r.refs.remotes.iter().find(|r| &*r.name == "origin").or(self.r.refs.remotes.first());
        if let Some(r) = remote
            && let (Some(url), Some(host)) =
                (ffwd_core::web::commit_url(&r.fetch_url, &sha), ffwd_core::web::host(&r.fetch_url))
        {
            v.push(item(format!("Open on {host}"), MenuAct::OpenUrl(url.clone())));
            v.push(item("Copy web link", MenuAct::Copy(url)));
        }
        v.push(item("Copy as patch", MenuAct::CopyPatch(PatchSrc::Commit(sha.clone()))));
        v.push(item("Save as patch…", MenuAct::Name(NameAct::SavePatch { src: PatchSrc::Commit(sha.clone()) })));
        if let Some((state, url)) = self.r.ci.get(&c.id) {
            let what = match state {
                ffwd_core::hosting::Ci::Passed => "Open the CI run (passed)",
                ffwd_core::hosting::Ci::Running => "Open the CI run (running)",
                ffwd_core::hosting::Ci::Failed => "Open the CI run (failed)",
            };
            v.push(item(what, MenuAct::OpenUrl(url.clone())));
        }
        // Compare (FF-33): mark this one, or compare with the marked one.
        v.push(sep());
        match self.r.compare_mark {
            Some(m) if m != c.id => {
                v.push(item(format!("Compare with {}", m.short(8)), MenuAct::Compare(m, c.id)));
                v.push(item("Mark for comparison instead", MenuAct::CompareMark(c.id)));
            }
            _ => v.push(item("Mark for comparison", MenuAct::CompareMark(c.id))),
        }
        if let Some((a, b)) = self.r.compare {
            v.push(item("Swap comparison order", MenuAct::Compare(b, a)));
        }
        v.push(sep());
        v.push(item("Copy SHA", MenuAct::Copy(sha.clone())));
        v.push(item("Copy subject", MenuAct::Copy(c.subject.to_string())));
        v
    }

    /// Shows what changed from `a` to `b` in the details pane (FF-33).
    pub(super) fn start_compare(&mut self, a: ffwd_core::Oid, b: ffwd_core::Oid) {
        let (Some(g), Some(t)) = (self.git_for_tab(), self.tabs.get(self.cur_tab)) else { return };
        self.r.seq += 1;
        let seq = self.r.seq;
        self.r.compare = Some((a, b));
        self.r.diff = None;
        let sink = (self.sinks)(t.id);
        crate::work::spawn("compare", &sink, move |sink| {
            let result = ffwd_core::diff::compare(&g, a, b).map(Box::new).map_err(|e| e.to_string());
            sink(Msg::Compared { seq, result });
        });
        self.damage.rects.push(self.lay.details);
    }

    /// The menu for a sidebar ref (`b:name`, `r:remote/name`, `t:name`).
    pub(super) fn ref_menu(&self, key: &str) -> Vec<MenuItem> {
        if self.r.side_multi.contains(key) && self.side_multi_live().len() > 1 {
            return self.batch_menu();
        }
        let mut v = self.ref_menu_one(key);
        // Every ref and stash can join a batch, by click too (FF-36).
        let pickable = ["b:", "t:", "s:"].iter().any(|p| key.starts_with(p))
            || key.strip_prefix("r:").is_some_and(|n| !n.ends_with('/'));
        if pickable {
            let pick = if cfg!(target_os = "macos") {
                "Select for several (Cmd+click)"
            } else {
                "Select for several (Ctrl+click)"
            };
            let label = if self.r.side_multi.contains(key) { "Unselect" } else { pick };
            v.push(sep());
            v.push(item(label, MenuAct::SidePick(key.into())));
        }
        let all = match key {
            "h:tags" => Some(("Select all tags", "t:")),
            "h:branches" => Some(("Select all branches", "b:")),
            "h:stashes" => Some(("Select all stashes", "s:")),
            _ => None,
        };
        if let Some((label, prefix)) = all {
            if !v.is_empty() {
                v.push(sep());
            }
            v.push(item(label, MenuAct::SidePickAll(prefix)));
        }
        v
    }

    /// The menu for several picked refs and stashes (FF-36): push them,
    /// delete them here or on the remote, drop the stashes.
    fn batch_menu(&self) -> Vec<MenuItem> {
        use ffwd_core::refops::count_refs;
        let head = self.head_name();
        let mut branches = Vec::new();
        let mut tags = Vec::new();
        let mut stashes = Vec::new();
        let mut remote: std::collections::BTreeMap<String, Vec<String>> = Default::default();
        for k in &self.side_multi_live() {
            if let Some(b) = k.strip_prefix("b:") {
                branches.push(b.to_string());
            } else if let Some(t) = k.strip_prefix("t:") {
                tags.push(t.to_string());
            } else if let Some(s) = k.strip_prefix("s:") {
                stashes.push(s.to_string());
            } else if let Some((rm, b)) = k.strip_prefix("r:").and_then(|n| n.split_once('/'))
                && b != "HEAD"
            {
                remote.entry(rm.to_string()).or_default().push(b.to_string());
            }
        }
        let list = |v: &[String]| {
            let mut s = v.iter().take(12).cloned().collect::<Vec<_>>().join(", ");
            if v.len() > 12 {
                s += &format!(" and {} more", v.len() - 12);
            }
            s
        };
        let mut v = Vec::new();
        let local = count_refs(branches.len(), tags.len());
        if (!branches.is_empty() || !tags.is_empty())
            && let Some(rm) = self.default_remote()
        {
            v.push(item(
                format!("Push {local} to {rm}"),
                MenuAct::Run(RefOp::PushMany(rm.clone(), branches.clone(), tags.clone())),
            ));
            if !tags.is_empty() {
                let n = count_refs(0, tags.len());
                v.push(danger(
                    format!("Delete {n} on {rm}…"),
                    MenuAct::Ask(Box::new(Ask {
                        title: format!("Delete {n} on {rm}?"),
                        text: format!(
                            "{} are removed from the remote for everyone. Your local tags stay, so pushing them again brings them back.",
                            list(&tags)
                        ),
                        choices: vec![(format!("Delete on {rm}"), RefOp::DeleteRemoteMany(rm, vec![], tags.clone()), true)],
                    })),
                ));
            }
        }
        for (rm, bs) in &remote {
            let n = count_refs(bs.len(), 0);
            v.push(danger(
                format!("Delete {n} on {rm}…"),
                MenuAct::Ask(Box::new(Ask {
                    title: format!("Delete {n} on {rm}?"),
                    text: format!(
                        "{} are removed from the remote {rm} for everyone. ffwd names the commands that push them back from your copies.",
                        list(bs)
                    ),
                    choices: vec![(format!("Delete on {rm}"), RefOp::DeleteRemoteMany(rm.clone(), bs.clone(), vec![]), true)],
                })),
            ));
        }
        // The checked-out branch is never deleted.
        let deletable: Vec<String> = branches.iter().filter(|b| head.as_deref() != Some(b.as_str())).cloned().collect();
        if !deletable.is_empty() || !tags.is_empty() {
            let n = count_refs(deletable.len(), tags.len());
            let names: Vec<String> = deletable.iter().chain(&tags).cloned().collect();
            let mut text = format!(
                "{} are deleted in this repository only; pushed copies stay on the remote. ffwd names the commands that restore them.",
                list(&names)
            );
            if deletable.len() < branches.len() {
                text += " The checked-out branch stays.";
            }
            let mut choices =
                vec![(format!("Delete {n}"), RefOp::DeleteMany(deletable.clone(), tags.clone(), false), true)];
            if !deletable.is_empty() {
                text += " Branches with commits on no other branch are kept, unless you delete them anyway.";
                choices.push(("Delete, unmerged too".into(), RefOp::DeleteMany(deletable, tags.clone(), true), true));
            }
            v.push(danger(
                format!("Delete {n}…"),
                MenuAct::Ask(Box::new(Ask { title: format!("Delete {n}?"), text, choices })),
            ));
        }
        if !stashes.is_empty() {
            let n = stashes.len();
            v.push(danger(
                format!("Drop {n} stashes…"),
                MenuAct::Ask(Box::new(Ask {
                    title: format!("Drop {n} stashes?"),
                    text: format!(
                        "{} are removed from the stash list. ffwd keeps each commit under refs/ffwd/backup/ and names the commands that bring them back.",
                        list(&stashes)
                    ),
                    choices: vec![(format!("Drop {n} stashes"), RefOp::StashDropMany(stashes), true)],
                })),
            ));
        }
        if !v.is_empty() {
            v.push(sep());
        }
        v.push(item("Clear the selection", MenuAct::SidePickClear));
        v
    }

    /// The picked refs and stashes that still exist (a reload or another
    /// tool may have removed some).
    pub(super) fn side_multi_live(&self) -> Vec<String> {
        let refs = &self.r.refs;
        self.r
            .side_multi
            .iter()
            .filter(|k| {
                let has = |kind: RefKind, n: &str| refs.refs.iter().any(|r| r.kind == kind && &*r.name == n);
                let n = k.get(2..).unwrap_or("");
                match k.get(..2) {
                    Some("b:") => has(RefKind::Local, n),
                    Some("t:") => has(RefKind::Tag, n),
                    Some("r:") => has(RefKind::Remote, n),
                    Some("s:") => refs.stashes.iter().any(|s| &*s.0 == n),
                    _ => false,
                }
            })
            .cloned()
            .collect()
    }

    fn ref_menu_one(&self, key: &str) -> Vec<MenuItem> {
        let head = self.head_name();
        let mut v = Vec::new();
        let merge_rebase = |v: &mut Vec<MenuItem>, name: &str| {
            if let Some(h) = &head
                && h != name
            {
                v.push(item(format!("Merge {name} into {h}…"), MenuAct::Ask(Box::new(self.merge_ask(name, name, h)))));
                v.push(item(
                    format!("Rebase {h} onto {name}…"),
                    MenuAct::Ask(Box::new(self.rebase_ask(name, name, h))),
                ));
            }
        };
        if let Some(name) = key.strip_prefix("b:") {
            let is_head = head.as_deref() == Some(name);
            if !is_head {
                v.push(item(format!("Checkout {name}"), MenuAct::Run(RefOp::Checkout(name.into()))));
            }
            merge_rebase(&mut v, name);
            v.push(sep());
            v.push(item(
                format!("Create branch from {name}…"),
                MenuAct::Name(NameAct::CreateBranch { at: name.into() }),
            ));
            v.push(item("Rename…", MenuAct::Name(NameAct::Rename { old: name.into() })));
            // Track a remote branch, or change which one (FF-36).
            let current = self
                .r
                .refs
                .refs
                .iter()
                .find(|r| r.kind == RefKind::Local && &*r.name == name)
                .and_then(|r| r.upstream.as_deref().map(str::to_string));
            let label = if current.is_some() { "Change upstream…" } else { "Set upstream…" };
            let current =
                current.unwrap_or_else(|| format!("{}/{name}", self.default_remote().unwrap_or("origin".into())));
            v.push(item(label, MenuAct::Name(NameAct::SetUpstream { branch: name.into(), current })));
            // An upstream deleted on the remote (FF-36): stop tracking it.
            if let Some(r) =
                self.r.refs.refs.iter().find(|r| r.kind == RefKind::Local && &*r.name == name && r.upstream_gone)
            {
                let up = r.upstream.as_deref().unwrap_or("its upstream").to_string();
                v.push(item(format!("Stop tracking {up} (gone)"), MenuAct::Run(RefOp::UnsetUpstream(name.into()))));
            }
            if let Some(rm) = self.default_remote() {
                v.push(item(format!("Push {name} to {rm}"), MenuAct::Run(RefOp::PushBranch(rm, name.into()))));
            }
            if !is_head {
                v.push(danger(format!("Delete {name}…"), MenuAct::Ask(Box::new(self.delete_ask(name)))));
            }
            v.push(sep());
            v.push(item("Copy name", MenuAct::Copy(name.into())));
        } else if let Some(name) = key.strip_prefix("r:").filter(|n| !n.ends_with('/')) {
            let (rm, branch) =
                name.split_once('/').map_or((String::new(), name.to_string()), |(a, b)| (a.to_string(), b.to_string()));
            v.push(item(
                format!("Checkout {name} (local tracking branch)"),
                MenuAct::Run(RefOp::Checkout(name.into())),
            ));
            merge_rebase(&mut v, name);
            v.push(sep());
            v.push(item(
                format!("Create branch from {name}…"),
                MenuAct::Name(NameAct::CreateBranch { at: name.into() }),
            ));
            if !rm.is_empty() && branch != "HEAD" {
                v.push(sep());
                v.push(danger(
                    format!("Delete {branch} on {rm}…"),
                    MenuAct::Ask(Box::new(Ask {
                        title: format!("Delete {branch} on {rm}?"),
                        text: format!(
                            "The branch {branch} is removed from the remote {rm} for everyone. ffwd names the command that pushes it back from your copy ({name})."
                        ),
                        choices: vec![(format!("Delete on {rm}"), RefOp::DeleteRemoteBranch(rm.clone(), branch.clone()), true)],
                    })),
                ));
            }
            v.push(item("Copy name", MenuAct::Copy(name.into())));
        } else if let Some(name) = key.strip_prefix("r:").and_then(|n| n.strip_suffix('/')) {
            let url = self.r.refs.remotes.iter().find(|r| &*r.name == name).map(|r| r.fetch_url.to_string());
            v.push(item(format!("Fetch {name}"), MenuAct::Run(RefOp::RemoteFetch(name.into()))));
            v.push(item("Test connection", MenuAct::Run(RefOp::RemoteTest(name.into()))));
            v.push(sep());
            if let Some(u) = &url {
                v.push(item("Edit URL…", MenuAct::Name(NameAct::RemoteUrl { name: name.into(), old: u.clone() })));
            }
            v.push(item("Rename…", MenuAct::Name(NameAct::RenameRemote { old: name.into() })));
            v.push(danger(
                format!("Remove {name}…"),
                MenuAct::Ask(Box::new(Ask {
                    title: format!("Remove the remote {name}?"),
                    text: format!(
                        "ffwd forgets {name} and its remote-tracking branches ({name}/…). Nothing on the server changes, and ffwd names the command that adds it back."
                    ),
                    choices: vec![(format!("Remove {name}"), RefOp::RemoteRemove(name.into()), true)],
                })),
            ));
            v.push(sep());
            v.push(item("Add remote…", MenuAct::Name(NameAct::AddRemoteUrl)));
            if let Some(u) = url {
                v.push(item("Copy URL", MenuAct::Copy(u)));
            }
        } else if let Some(name) = key.strip_prefix("m:") {
            let Some(s) = self.r.refs.submodules.iter().find(|s| &*s.name == name).cloned() else { return v };
            let path = s.path.to_string();
            if s.state != ffwd_core::submodules::State::Uninitialized {
                v.push(item("Open in a new tab", MenuAct::OpenSubmodule(name.into())));
            }
            v.push(item(format!("Update {path}"), MenuAct::Run(RefOp::SubUpdate(Some(path.clone())))));
            v.push(sep());
            v.push(item("Move…", MenuAct::Name(NameAct::MoveSubmodule { path: path.clone() })));
            if s.url.starts_with("../") || s.url.starts_with("./") {
                v.push(item("Make the URL absolute", MenuAct::Run(RefOp::SubAbsolute(name.into()))));
            } else {
                v.push(item("Make the URL relative", MenuAct::Run(RefOp::SubRelative(Some(name.into())))));
            }
            v.push(sep());
            v.push(item("Add submodule…", MenuAct::Name(NameAct::AddSubmoduleUrl)));
            v.push(item("Copy path", MenuAct::Copy(path)));
            v.push(item("Copy URL", MenuAct::Copy(s.url.to_string())));
        } else if let Some(path) = key.strip_prefix("w:") {
            // A worktree (FF-45): open it, a terminal or the agent's CLI
            // there, remove it (Git refuses one with changes).
            let p = PathBuf::from(path);
            let main = self.r.refs.worktrees.first().is_some_and(|w| **w == *path);
            v.push(item("Open in a new tab", MenuAct::OpenTab(p.clone())));
            v.push(item("Open terminal here", MenuAct::OpenTerminal(p.clone())));
            v.push(item(open_folder_label(), MenuAct::OpenFolder(p.clone())));
            v.push(sep());
            v.extend(self.agent_terminal_menu(Some(p), false));
            v.push(sep());
            v.push(item("Copy path", MenuAct::Copy(path.to_string())));
            if !main {
                v.push(danger(
                    "Remove worktree…",
                    MenuAct::Ask(Box::new(Ask {
                        title: format!("Remove the worktree {path}?"),
                        text: "The folder is deleted; its branch and commits stay. Git refuses when it has changes or untracked files, so nothing unsaved is lost.".into(),
                        choices: vec![("Remove worktree".into(), RefOp::WorktreeRemove(path.to_string()), true)],
                    })),
                ));
            }
        } else if key == "h:submodules" {
            v.push(item("Update all submodules", MenuAct::Run(RefOp::SubUpdate(None))));
            v.push(item("Make all URLs relative", MenuAct::Run(RefOp::SubRelative(None))));
            v.push(item("Add submodule…", MenuAct::Name(NameAct::AddSubmoduleUrl)));
        } else if let Some(n) = key.strip_prefix("p:").and_then(|n| n.parse::<u32>().ok()) {
            let pr = self
                .r
                .prs
                .as_ref()
                .and_then(|r| r.as_ref().ok())
                .and_then(|v| v.iter().find(|p| p.number == n))
                .cloned();
            let remote = self
                .r
                .refs
                .remotes
                .iter()
                .find(|r| ffwd_core::web::host(&r.fetch_url).is_some_and(|h| h == "github.com"))
                .map(|r| r.name.to_string())
                .unwrap_or_else(|| "origin".into());
            v.push(item(format!("Check out as pr/{n}"), MenuAct::Run(RefOp::PrCheckout(remote, n))));
            if let Some(p) = pr {
                v.push(item("Open in the browser", MenuAct::OpenUrl(p.url.to_string())));
                v.push(sep());
                v.push(item("Copy link", MenuAct::Copy(p.url.to_string())));
                v.push(item("Copy branch name", MenuAct::Copy(p.head.to_string())));
            }
        } else if key == "h:prs" {
            v.push(item("Create pull request on GitHub", MenuAct::CreatePr));
        } else if key == "h:branches" {
            let recent = self.settings.get("sidebar.branch_order") == Some("recent");
            let mark = |on: bool, t: &str| if on { format!("✓ {t}") } else { t.to_string() };
            v.push(item(mark(!recent, "Sort by name"), MenuAct::BranchOrder(false)));
            v.push(item(mark(recent, "Sort by most recent commit"), MenuAct::BranchOrder(true)));
        } else if key == "h:remotes" {
            v.push(item("Add remote…", MenuAct::Name(NameAct::AddRemoteUrl)));
            v.push(item("Fetch all remotes", MenuAct::Run(RefOp::Sync(ffwd_core::ops::Op::Fetch))));
        } else if let Some(name) = key.strip_prefix("s:") {
            let msg = self.r.refs.stashes.iter().find(|s| &*s.0 == name).map_or(String::new(), |s| s.1.to_string());
            v.push(item(format!("Apply {name}"), MenuAct::Run(RefOp::StashApply(name.into(), false))));
            v.push(item(format!("Pop {name} (apply, then drop)"), MenuAct::Run(RefOp::StashApply(name.into(), true))));
            v.push(sep());
            v.push(item("Rename…", MenuAct::Name(NameAct::StashRename { name: name.into(), old: msg.clone() })));
            v.push(danger(
                format!("Drop {name}…"),
                MenuAct::Ask(Box::new(Ask {
                    title: format!("Drop {name}?"),
                    text: format!(
                        "\"{msg}\" is removed from the stash list. ffwd keeps its commit under refs/ffwd/backup/ and names the command that brings it back."
                    ),
                    choices: vec![(format!("Drop {name}"), RefOp::StashDrop(name.into()), true)],
                })),
            ));
            v.push(sep());
            v.push(item("Copy as patch", MenuAct::CopyPatch(PatchSrc::Stash(name.into()))));
            v.push(item("Copy message", MenuAct::Copy(msg)));
        } else if let Some(name) = key.strip_prefix("t:") {
            v.push(item(format!("Checkout {name} (detached)"), MenuAct::Run(RefOp::Checkout(name.into()))));
            merge_rebase(&mut v, name);
            v.push(sep());
            v.push(item(
                format!("Create branch from {name}…"),
                MenuAct::Name(NameAct::CreateBranch { at: name.into() }),
            ));
            if let Some(rm) = self.default_remote() {
                v.push(item(format!("Push tag {name} to {rm}"), MenuAct::Run(RefOp::PushTag(rm.clone(), name.into()))));
                v.push(danger(
                    format!("Delete tag {name} on {rm}…"),
                    MenuAct::Ask(Box::new(Ask {
                        title: format!("Delete tag {name} on {rm}?"),
                        text: "The tag is removed from the remote for everyone; your local tag stays, so pushing it again brings it back.".into(),
                        choices: vec![(format!("Delete on {rm}"), RefOp::DeleteRemoteTag(rm, name.into()), true)],
                    })),
                ));
            }
            v.push(danger(
                format!("Delete tag {name}…"),
                MenuAct::Ask(Box::new(Ask {
                    title: format!("Delete tag {name}?"),
                    text: "The tag is removed from this repository only; a pushed tag stays on the remote. ffwd names the command that restores it.".into(),
                    choices: vec![(format!("Delete {name}"), RefOp::DeleteTag(name.into()), true)],
                })),
            ));
            v.push(item("Copy name", MenuAct::Copy(name.into())));
        }
        v
    }

    fn merge_ask(&self, target: &str, what: &str, head: &str) -> Ask {
        Ask {
            title: format!("Merge {what} into {head}"),
            text: format!(
                "Fast-forward when {head} has no commits of its own, otherwise a merge commit. If conflicts appear the merge stops, and git merge --abort undoes it."
            ),
            choices: vec![
                ("Merge".into(), RefOp::Merge(target.into(), MergeMode::Default), false),
                ("Fast-forward only".into(), RefOp::Merge(target.into(), MergeMode::FastForwardOnly), false),
                ("Always a merge commit".into(), RefOp::Merge(target.into(), MergeMode::NoFastForward), false),
                ("Squash".into(), RefOp::Merge(target.into(), MergeMode::Squash), false),
            ],
        }
    }

    fn rebase_ask(&self, target: &str, what: &str, head: &str) -> Ask {
        Ask {
            title: format!("Rebase {head} onto {what}?"),
            text: format!(
                "{head}'s own commits are rewritten on top of {what}. The old tip is kept as a backup ref (refs/ffwd/backup/…). Don't rebase commits others already pulled."
            ),
            choices: vec![(format!("Rebase {head}"), RefOp::Rebase(target.into()), false)],
        }
    }

    pub(super) fn reset_ask(&self, sha: &str, short: &str, head: &str) -> Ask {
        Ask {
            title: format!("Reset {head} to {short}?"),
            text: "Soft keeps every change staged. Mixed keeps them as unstaged changes. Hard throws away staged and unstaged changes too. The current tip is kept as a backup ref first, so the commits can be restored.".into(),
            choices: vec![
                ("Mixed".into(), RefOp::Reset(sha.into(), ResetMode::Mixed), false),
                ("Soft".into(), RefOp::Reset(sha.into(), ResetMode::Soft), false),
                ("Hard (lose local changes)".into(), RefOp::Reset(sha.into(), ResetMode::Hard), true),
            ],
        }
    }

    fn delete_ask(&self, name: &str) -> Ask {
        Ask {
            title: format!("Delete branch {name}?"),
            text: "Only the local branch is deleted; its commits stay reachable from other branches. ffwd names the command that restores it.".into(),
            choices: vec![(format!("Delete {name}"), RefOp::DeleteBranch(name.into(), false), true)],
        }
    }

    pub(super) fn open_menu(&mut self, x: i32, y: i32, items: Vec<MenuItem>) {
        if items.is_empty() {
            return;
        }
        self.palette = None;
        let ui = self.f.ui;
        let longest = items.iter().map(|i| self.te.measure(ui, self.pf(12.5), &i.label)).fold(0.0f32, f32::max);
        let w = ((longest + self.pf(26.0 + 20.0 + 12.0)) as i32).clamp(self.px(300.0), self.px(460.0));
        self.menu = Some(Menu { x, y, w, items, sel: None });
        let r = self.menu_rect();
        self.damage.rects.push(r);
    }

    pub(super) fn close_menu(&mut self) {
        if self.menu.is_some() {
            let r = self.menu_rect();
            self.menu = None;
            self.damage.rects.push(r);
        }
    }

    fn menu_item_h(&self, it: &MenuItem) -> i32 {
        if it.act.is_none() { self.px(9.0) } else { self.px(30.0) }
    }

    /// The menu's rectangle, kept inside the window.
    pub(super) fn menu_rect(&self) -> Rect {
        let Some(m) = &self.menu else { return Rect::default() };
        let h: i32 = m.items.iter().map(|i| self.menu_item_h(i)).sum::<i32>() + self.px(12.0);
        let w = m.w;
        let x = m.x.min(self.w as i32 - w - self.px(6.0)).max(0);
        let y = m.y.min(self.h as i32 - h - self.px(6.0)).max(0);
        Rect::new(x, y, w, h)
    }

    pub(super) fn menu_item_rects(&self) -> Vec<(usize, Rect)> {
        let Some(m) = &self.menu else { return Vec::new() };
        let r = self.menu_rect();
        let mut y = r.y + self.px(6.0);
        let mut v = Vec::new();
        for (i, it) in m.items.iter().enumerate() {
            let h = self.menu_item_h(it);
            if it.act.is_some() {
                v.push((i, Rect::new(r.x + self.px(6.0), y, r.w - self.px(12.0), h)));
            }
            y += h;
        }
        v
    }

    pub(super) fn menu_activate(&mut self, i: usize) {
        let act = self.menu.as_ref().and_then(|m| m.items.get(i)).and_then(|it| it.act.clone());
        self.close_menu();
        match act {
            Some(MenuAct::Run(op)) => self.run_refop(op),
            Some(MenuAct::OpenFile(p, editor)) => self.open_external(p, editor, None),
            Some(MenuAct::OpenFileAt(p, line)) => self.open_external(p, true, Some(line)),
            Some(MenuAct::CompareMark(id)) => {
                self.r.compare_mark = Some(id);
                self.set_notice(format!(
                    "{} is marked: right-click another commit and choose Compare with {}",
                    id.short(8),
                    id.short(8)
                ));
            }
            Some(MenuAct::Compare(a, b)) => self.start_compare(a, b),
            Some(MenuAct::StageToggle(_)) => self.toggle_selected_change(),
            Some(MenuAct::Discard) => self.discard_selected(),
            Some(MenuAct::Ignore) => self.ignore_selected(),
            Some(MenuAct::ShowFile(p)) => crate::platform::show_file(p),
            Some(MenuAct::DiffTool(path, of)) => self.run_tool(path, Some(of)),
            Some(MenuAct::MergeTool(path)) => self.run_tool(path, None),
            Some(MenuAct::CloseTab(i)) => self.close_tab(i),
            Some(MenuAct::CloseOtherTabs(i)) => self.close_other_tabs(i),
            Some(MenuAct::OpenFolder(p)) => crate::platform::open_folder(p),
            Some(MenuAct::OpenTerminal(p)) => crate::platform::open_terminal(p),
            Some(MenuAct::OpenUrl(u)) => crate::platform::open_url(u),
            Some(MenuAct::DiffShowWs(on)) => self.set_diff_view(Some(on), None),
            Some(MenuAct::DiffTab(t)) => self.set_diff_view(None, Some(t)),
            Some(MenuAct::HideUntracked(h)) => self.set_hide_untracked(h),
            Some(MenuAct::ShowIgnored(s)) => self.set_show_ignored(s),
            Some(MenuAct::DiffWrap(on)) => self.set_diff_wrap(on),
            Some(MenuAct::DiffBar(h)) => self.apply_diff_bar(h),
            Some(MenuAct::CodeSize(s)) => self.set_code_size(s),
            Some(MenuAct::TabColor(i, c)) => self.set_tab_color(i, c),
            Some(MenuAct::BranchOrder(recent)) => {
                self.settings.set_string("sidebar.branch_order", if recent { "recent" } else { "name" });
                self.rebuild_side();
                self.damage.rects.push(self.lay.side);
            }
            Some(MenuAct::CopyPatch(src)) => self.make_patch(src, None),
            Some(MenuAct::CreatePr) => self.create_pr(),
            Some(MenuAct::ShowCommit(sha)) => {
                if let Some(&row) = ffwd_core::Oid::from_hex(sha.as_bytes()).and_then(|id| self.r.by_id.get(&id)) {
                    self.leave_reflog();
                    self.side_select("all");
                    self.select(row as usize);
                }
            }
            Some(MenuAct::AgentTerminal { path, worktree, sandboxed, preset }) => {
                self.agent_terminal(path, worktree, sandboxed, preset)
            }
            Some(MenuAct::OpenTab(p)) => self.open_tab(p),
            Some(MenuAct::SettingChoice(row, i)) => self.set_choice(row, i),
            Some(MenuAct::SettingScope(repo)) => self.set_settings_scope(repo),
            Some(MenuAct::SidePick(key)) => {
                if let Some(i) = self.r.side.iter().position(|it| it.key == key) {
                    self.side_pick(i, false);
                }
            }
            Some(MenuAct::SidePickAll(prefix)) => {
                let keys: Vec<String> = self
                    .r
                    .side
                    .iter()
                    .filter(|it| super::sidebar::multi_kind(it.kind) && it.key.starts_with(prefix))
                    .map(|it| it.key.clone())
                    .collect();
                let n = keys.len();
                self.r.side_multi.extend(keys);
                self.set_notice(format!("{n} selected: right-click one of them for push, delete or drop"));
                self.damage.rects.push(self.lay.side);
            }
            Some(MenuAct::SidePickClear) => {
                self.r.side_multi.clear();
                self.damage.rects.push(self.lay.side);
            }
            Some(MenuAct::Scope(head_only)) => {
                if self.tabs.get(self.cur_tab).is_some_and(|t| t.head_only != head_only) {
                    self.toggle_scope();
                }
            }
            Some(MenuAct::ShowStashes(on)) => self.set_show_stashes(on),
            Some(MenuAct::RebaseFrom(sha)) => self.open_rebase(sha),
            Some(MenuAct::OpenSubmodule(name)) => self.open_submodule(&name),
            Some(MenuAct::Ask(a)) => self.open_dialog(DialogKind::Ask(*a)),
            Some(MenuAct::Name(NameAct::SavePatch { src })) => {
                let path = self.patch_path(&src);
                self.open_dialog(DialogKind::Name {
                    act: NameAct::SavePatch { src },
                    field: TextField::new(&path.to_string_lossy()),
                    checkout: false,
                });
            }
            Some(MenuAct::Name(n)) => {
                let field = match &n {
                    NameAct::Rename { old }
                    | NameAct::StashRename { old, .. }
                    | NameAct::RemoteUrl { old, .. }
                    | NameAct::RenameRemote { old }
                    | NameAct::SetUpstream { current: old, .. }
                    | NameAct::RenameTab { old, .. }
                    | NameAct::MoveSubmodule { path: old } => TextField::new(old),
                    _ => TextField::default(),
                };
                let checkout = matches!(n, NameAct::CreateBranch { .. } | NameAct::StashSave);
                self.open_dialog(DialogKind::Name { act: n, field, checkout });
            }
            Some(MenuAct::Copy(t)) => {
                crate::platform::set_clipboard(&t);
                self.set_notice(format!("Copied {t}"));
            }
            None => {}
        }
    }

    pub(super) fn menu_key(&mut self, k: Key) {
        let Some(m) = &mut self.menu else { return };
        let actionable: Vec<usize> =
            m.items.iter().enumerate().filter(|(_, i)| i.act.is_some()).map(|(i, _)| i).collect();
        let pos = m.sel.and_then(|s| actionable.iter().position(|&a| a == s));
        match k {
            Key::Esc => return self.close_menu(),
            Key::Down | Key::Tab => m.sel = Some(actionable[pos.map_or(0, |p| (p + 1) % actionable.len())]),
            Key::Up | Key::BackTab => {
                m.sel = Some(
                    actionable[pos.map_or(actionable.len() - 1, |p| (p + actionable.len() - 1) % actionable.len())],
                )
            }
            Key::Enter | Key::Char(' ') => {
                if let Some(s) = m.sel {
                    return self.menu_activate(s);
                }
            }
            _ => {}
        }
        let r = self.menu_rect();
        self.damage.rects.push(r);
    }

    /// A click while the menu is open: on an item it runs, elsewhere the
    /// menu closes.
    pub(super) fn menu_click(&mut self, x: i32, y: i32) {
        match self.menu_item_rects().into_iter().find(|(_, r)| r.contains(x, y)) {
            Some((i, _)) => self.menu_activate(i),
            None => self.close_menu(),
        }
    }

    pub(super) fn open_dialog(&mut self, kind: DialogKind) {
        self.dialog = Some(Dialog { kind, sel: 0, error: None, busy: false, hits: Vec::new() });
        self.damage.full = true;
    }

    pub(super) fn close_dialog(&mut self) {
        if self.dialog.take().is_some() {
            self.damage.full = true;
        }
    }

    pub(super) fn dialog_rect(&self) -> Rect {
        let w = self.px(560.0).min(self.w as i32 - self.px(40.0));
        let h = match self.dialog.as_ref().map(|d| &d.kind) {
            // The stash dialog has a line of text above its field.
            Some(DialogKind::Name {
                act:
                    NameAct::StashSave
                    | NameAct::AddRemoteUrl
                    | NameAct::AddSubmoduleUrl
                    | NameAct::MoveSubmodule { .. }
                    | NameAct::CloneUrl
                    | NameAct::InitDir
                    | NameAct::ApplyPatchFile,
                ..
            }) => self.px(250.0),
            Some(DialogKind::Name { .. }) => self.px(210.0),
            _ => self.px(230.0),
        };
        Rect::new((self.w as i32 - w) / 2, (self.h as i32 - h) / 3, w, h)
    }

    fn dialog_confirm(&mut self, choice: usize) {
        let Some(d) = &mut self.dialog else { return };
        if d.busy {
            return;
        }
        // Adding a submodule asks for the URL, then the folder; then, when
        // the URL can be relative to this repository's remote, maybe asks.
        if let DialogKind::Name { act: NameAct::AddSubmoduleUrl, field, .. } = &d.kind {
            let url = field.text().trim().to_string();
            if url.is_empty() {
                d.error = Some("Type the URL first.".into());
                let r = self.dialog_rect();
                self.damage.rects.push(r);
                return;
            }
            let folder = url
                .trim_end_matches('/')
                .rsplit(['/', ':', '\\'])
                .next()
                .unwrap_or("")
                .trim_end_matches(".git")
                .to_string();
            self.open_dialog(DialogKind::Name {
                act: NameAct::AddSubmodulePath { url },
                field: TextField::new(&folder),
                checkout: false,
            });
            return;
        }
        if let DialogKind::Name { act: NameAct::AddSubmodulePath { url }, field, .. } = &d.kind {
            let (url, path) = (url.clone(), field.text().trim().to_string());
            return self.add_submodule(url, path);
        }
        // Cloning asks for the URL, then the folder (suggested).
        if let DialogKind::Name { act: NameAct::CloneUrl, field, .. } = &d.kind {
            let url = field.text().trim().to_string();
            if url.is_empty() {
                d.error = Some("Type the URL first.".into());
                let r = self.dialog_rect();
                self.damage.rects.push(r);
                return;
            }
            let dest = self.default_repo_folder().join(ffwd_core::clone::folder_for(&url));
            self.open_dialog(DialogKind::Name {
                act: NameAct::CloneDest { url },
                field: TextField::new(&dest.to_string_lossy()),
                checkout: false,
            });
            return;
        }
        if let DialogKind::Name { act: NameAct::CloneDest { url }, field, .. } = &d.kind {
            let (url, dest) = (url.clone(), PathBuf::from(field.text().trim()));
            self.close_dialog();
            return self.start_clone(url, dest);
        }
        if let DialogKind::Name { act: NameAct::RenameTab { index, .. }, field, .. } = &d.kind {
            let (i, name) = (*index, field.text().to_string());
            self.close_dialog();
            return self.rename_tab(i, &name);
        }
        if let DialogKind::Name { act: NameAct::InitDir, field, .. } = &d.kind {
            let dir = PathBuf::from(field.text().trim());
            self.close_dialog();
            return self.start_init(dir);
        }
        // Patches: save to a file, or read one to apply (FF-46).
        if let DialogKind::Name { act: NameAct::SavePatch { src }, field, .. } = &d.kind {
            let (src, path) = (src.clone(), PathBuf::from(field.text().trim()));
            self.close_dialog();
            return self.make_patch(src, Some(path));
        }
        if let DialogKind::Name { act: NameAct::ApplyPatchFile, field, .. } = &d.kind {
            let path = PathBuf::from(field.text().trim());
            self.close_dialog();
            return self.check_patch_file(path);
        }
        // Adding a remote asks for the URL first, then a name it suggests.
        if let DialogKind::Name { act: NameAct::AddRemoteUrl, field, .. } = &d.kind {
            let url = field.text().trim().to_string();
            if url.is_empty() {
                d.error = Some("Type the URL or path first.".into());
                let r = self.dialog_rect();
                self.damage.rects.push(r);
                return;
            }
            let name = ffwd_core::remotes::suggest_name(&url, &self.r.refs.remotes);
            self.open_dialog(DialogKind::Name {
                act: NameAct::AddRemoteName { url },
                field: TextField::new(&name),
                checkout: false,
            });
            return;
        }
        let op = match &d.kind {
            DialogKind::Ask(a) => a.choices.get(choice).map(|c| c.1.clone()),
            DialogKind::Name { act, field, checkout } => {
                let n = field.text().trim().to_string();
                let ref_name =
                    matches!(act, NameAct::CreateBranch { .. } | NameAct::CreateTag { .. } | NameAct::Rename { .. });
                if n.is_empty() && !matches!(act, NameAct::StashSave) {
                    d.error = Some("Type a name first.".into());
                    None
                } else if let Some(e) = ref_name.then(|| ffwd_core::refs::check_ref_name(&n).err()).flatten() {
                    d.error = Some(e);
                    None
                } else if matches!(act, NameAct::SetUpstream { .. })
                    && !self.r.refs.refs.iter().any(|r| r.kind == RefKind::Remote && *r.name == *n)
                {
                    d.error = Some(format!("{n} is not a remote branch here; fetch first, or check the name."));
                    None
                } else {
                    Some(match act {
                        NameAct::CreateBranch { at } => RefOp::CreateBranch(n, at.clone(), *checkout),
                        NameAct::CreateTag { at } => RefOp::CreateTag(n, at.clone()),
                        NameAct::Rename { old } => RefOp::Rename(old.clone(), n),
                        NameAct::StashSave => RefOp::StashSave(
                            n,
                            ffwd_core::stash::SaveOptions { untracked: *checkout, keep_index: false },
                        ),
                        NameAct::StashRename { name, .. } => RefOp::StashRename(name.clone(), n),
                        NameAct::AddRemoteUrl => unreachable!("handled above"),
                        NameAct::AddRemoteName { url } => RefOp::RemoteAdd(n, url.clone()),
                        NameAct::RemoteUrl { name, .. } => RefOp::RemoteSetUrl(name.clone(), n),
                        NameAct::RenameRemote { old } => RefOp::RemoteRename(old.clone(), n),
                        NameAct::SetUpstream { branch, .. } => RefOp::SetUpstream(branch.clone(), n),
                        NameAct::RenameTab { .. } => unreachable!("handled above"),
                        NameAct::CloneUrl | NameAct::CloneDest { .. } | NameAct::InitDir => {
                            unreachable!("handled above")
                        }
                        NameAct::AddSubmoduleUrl | NameAct::AddSubmodulePath { .. } => unreachable!("handled above"),
                        NameAct::MoveSubmodule { path } => RefOp::SubMove(path.clone(), n),
                        NameAct::SavePatch { .. } | NameAct::ApplyPatchFile => unreachable!("handled above"),
                    })
                }
            }
        };
        match op {
            Some(op) => {
                d.busy = true;
                d.error = None;
                self.run_refop(op);
            }
            None => {
                let r = self.dialog_rect();
                self.damage.rects.push(r);
            }
        }
    }

    pub(super) fn dialog_key(&mut self, k: Key) {
        let Some(d) = &mut self.dialog else { return };
        let n = match &d.kind {
            DialogKind::Ask(a) => a.choices.len(),
            DialogKind::Name { .. } => 1,
        };
        match (&mut d.kind, k) {
            (_, Key::Esc) => return self.close_dialog(),
            (_, Key::Enter) => {
                let s = d.sel;
                return self.dialog_confirm(s);
            }
            (DialogKind::Ask(_), Key::Right | Key::Tab | Key::Down) => d.sel = (d.sel + 1) % n,
            (DialogKind::Ask(_), Key::Left | Key::BackTab | Key::Up) => d.sel = (d.sel + n - 1) % n,
            (DialogKind::Name { checkout, act: NameAct::CreateBranch { .. } | NameAct::StashSave, .. }, Key::Tab) => {
                *checkout = !*checkout
            }
            (DialogKind::Name { field, act, .. }, k) => {
                let ref_name =
                    matches!(act, NameAct::CreateBranch { .. } | NameAct::CreateTag { .. } | NameAct::Rename { .. });
                match k {
                    // Branch and tag names can't hold spaces: type "-" (FF-36).
                    Key::Char(' ') if ref_name => field.insert("-"),
                    Key::Char(c) => field.insert(c.encode_utf8(&mut [0; 4])),
                    Key::Backspace => field.backspace(),
                    Key::CtrlBackspace => field.delete_word(),
                    Key::Delete => field.delete(),
                    Key::Left => field.left(),
                    Key::Right => field.right(),
                    Key::Home => field.home(),
                    Key::End => field.end(),
                    Key::Ctrl('a') => field.select_all(),
                    Key::Ctrl('v') => {
                        if let Some(t) = crate::platform::clipboard() {
                            let line = t.lines().next().unwrap_or("");
                            if ref_name {
                                let dashed: Vec<&str> = line.split_whitespace().collect();
                                field.insert(&dashed.join("-"));
                            } else {
                                field.insert(line);
                            }
                        }
                    }
                    _ => {}
                }
                // Branch and tag names say what's wrong while typing.
                let text = field.text().trim().to_string();
                d.error =
                    (ref_name && !text.is_empty()).then(|| ffwd_core::refs::check_ref_name(&text).err()).flatten();
            }
            _ => {}
        }
        let r = self.dialog_rect();
        self.damage.rects.push(r);
    }

    pub(super) fn dialog_click(&mut self, x: i32, y: i32) {
        let r = self.dialog_rect();
        if !r.contains(x, y) {
            return self.close_dialog();
        }
        let hit = self.dialog.as_ref().and_then(|d| d.hits.iter().find(|h| h.0.contains(x, y)).map(|h| h.1));
        match hit {
            Some(DHit::Choice(i)) => self.dialog_confirm(i),
            Some(DHit::Cancel) => self.close_dialog(),
            Some(DHit::Toggle) => {
                if let Some(Dialog { kind: DialogKind::Name { checkout, .. }, .. }) = &mut self.dialog {
                    *checkout = !*checkout;
                }
                self.damage.rects.push(r);
            }
            _ => {}
        }
    }

    /// Opens submodule `name` in a new tab.
    pub(super) fn open_submodule(&mut self, name: &str) {
        let Some(s) = self.r.refs.submodules.iter().find(|s| &*s.name == name).cloned() else { return };
        let Some(root) = self.r.git.as_ref().map(|g| g.root.clone()) else { return };
        if s.state == ffwd_core::submodules::State::Uninitialized {
            return self.set_notice(format!("{} is not set up yet: choose Update in its menu first.", s.path));
        }
        self.open_tab(root.join(&*s.path));
    }

    /// Adds a submodule, proposing a relative URL when it can be one
    /// (setting `submodules.relative_urls`: ask, always or never).
    fn add_submodule(&mut self, url: String, path: String) {
        if path.is_empty() {
            if let Some(d) = &mut self.dialog {
                d.error = Some("Type the folder first.".into());
            }
            let r = self.dialog_rect();
            self.damage.rects.push(r);
            return;
        }
        let mode = self
            .current_repo_settings()
            .and_then(|r| r.get("submodules.relative_urls"))
            .or_else(|| self.settings.get("submodules.relative_urls"))
            .unwrap_or("ask")
            .to_string();
        let base = self.r.refs.remotes.iter().find(|r| &*r.name == "origin").or(self.r.refs.remotes.first());
        let rel = base.and_then(|b| ffwd_core::submodules::relative_url(&b.fetch_url, &url));
        match (rel, mode.as_str()) {
            (Some(rel), "always") => self.run_refop(RefOp::SubAdd(rel, path)),
            (Some(rel), "ask") => self.open_dialog(DialogKind::Ask(Ask {
                title: "Use a relative URL?".into(),
                text: format!(
                    "{url} lives on the same host as this repository's remote. The relative URL {rel} keeps working for forks, mirrors and ssh or https clones. (Setting submodules.relative_urls: ask, always, never.)"
                ),
                choices: vec![
                    (format!("Add with {rel}"), RefOp::SubAdd(rel.clone(), path.clone()), false),
                    ("Add with the URL as typed".into(), RefOp::SubAdd(url.clone(), path.clone()), false),
                ],
            })),
            _ => self.run_refop(RefOp::SubAdd(url, path)),
        }
    }

    /// Makes a patch on a worker: to the clipboard, or saved to `to`.
    pub(super) fn make_patch(&mut self, src: PatchSrc, to: Option<PathBuf>) {
        let (Some(g), Some(t)) = (self.git_for_tab(), self.tabs.get(self.cur_tab)) else { return };
        let sink = (self.sinks)(t.id);
        crate::work::spawn("patch", &sink, move |sink| {
            use ffwd_core::patches as p;
            let r = match &src {
                PatchSrc::Commit(sha) => p::commit_patch(&g, sha),
                PatchSrc::CommitFile(sha, path) => p::commit_file_patch(&g, sha, path),
                PatchSrc::Stash(n) => p::stash_patch(&g, n),
                PatchSrc::Working(staged) => p::working_patch(&g, *staged),
            };
            let r = r.and_then(|text| match to {
                Some(path) => {
                    if let Some(d) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
                        std::fs::create_dir_all(d).map_err(|e| format!("cannot create {}: {e}", d.display()))?;
                    }
                    std::fs::write(&path, &text).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
                    Ok((String::new(), Some(path)))
                }
                None => Ok((text, None)),
            });
            sink(Msg::PatchMade(r));
        });
    }

    pub(super) fn on_patch_made(&mut self, r: Result<(String, Option<PathBuf>), String>) {
        match r {
            Ok((_, Some(path))) => self.set_notice(format!("Saved the patch to {}", path.display())),
            Ok((text, None)) => {
                let n = ffwd_core::patches::files(&text).len();
                crate::platform::set_clipboard(&text);
                self.set_notice(format!(
                    "Copied a patch of {n} file{} to the clipboard",
                    if n == 1 { "" } else { "s" }
                ));
            }
            Err(e) => self.set_notice(format!("No patch: {e}")),
        }
    }

    /// Where a saved patch goes by default: the Downloads folder.
    pub(super) fn patch_path(&self, src: &PatchSrc) -> PathBuf {
        let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
        let dir = home.map(|h| h.join("Downloads")).unwrap_or_default();
        let name = match src {
            PatchSrc::Commit(sha) => {
                let subject = self.r.commits.iter().find(|c| c.id.to_string() == *sha).map(|c| c.subject.to_string());
                let slug: String = subject
                    .unwrap_or_default()
                    .chars()
                    .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
                    .collect::<String>()
                    .split('-')
                    .filter(|s| !s.is_empty())
                    .take(6)
                    .collect::<Vec<_>>()
                    .join("-");
                format!("{}-{slug}.patch", &sha[..sha.len().min(8)])
            }
            PatchSrc::CommitFile(sha, path) => {
                let file = path.rsplit('/').next().unwrap_or(path);
                let slug: String =
                    file.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' { c } else { '-' }).collect();
                format!("{}-{}.patch", &sha[..sha.len().min(8)], slug.trim_matches(['-', '.']))
            }
            PatchSrc::Stash(_) => "stash.patch".into(),
            PatchSrc::Working(true) => "staged.patch".into(),
            PatchSrc::Working(false) => "changes.patch".into(),
        };
        dir.join(name)
    }

    /// Checks a patch on a worker, then asks before applying it.
    pub(super) fn check_patch(&mut self, patch: String) {
        let (Some(g), Some(t)) = (self.git_for_tab(), self.tabs.get(self.cur_tab)) else { return };
        if patch.len() > ffwd_core::patches::MAX_PATCH {
            return self.set_notice("That patch is larger than 32 MB.".into());
        }
        let sink = (self.sinks)(t.id);
        crate::work::spawn("patch-check", &sink, move |sink| {
            let result = ffwd_core::patches::check(&g, &patch);
            sink(Msg::PatchChecked { patch, result });
        });
    }

    fn check_patch_file(&mut self, path: PathBuf) {
        let (Some(g), Some(t)) = (self.git_for_tab(), self.tabs.get(self.cur_tab)) else { return };
        let sink = (self.sinks)(t.id);
        crate::work::spawn("patch-read", &sink, move |sink| {
            let read = std::fs::metadata(&path)
                .map_err(|e| format!("cannot read {}: {e}", path.display()))
                .and_then(|m| {
                    if m.len() as usize > ffwd_core::patches::MAX_PATCH {
                        Err("That patch is larger than 32 MB".to_string())
                    } else {
                        std::fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))
                    }
                })
                .map(|b| String::from_utf8_lossy(&b).into_owned());
            match read {
                Ok(patch) => {
                    let result = ffwd_core::patches::check(&g, &patch);
                    sink(Msg::PatchChecked { patch, result });
                }
                Err(e) => sink(Msg::PatchChecked { patch: String::new(), result: Err(e) }),
            }
        });
    }

    pub(super) fn on_patch_checked(&mut self, patch: String, r: Result<Vec<String>, String>) {
        match r {
            Ok(files) => {
                let n = files.len();
                let list: Vec<String> = files.iter().take(6).cloned().collect();
                let more = if n > 6 { format!(" and {} more", n - 6) } else { String::new() };
                self.open_dialog(DialogKind::Ask(Ask {
                    title: "Apply the patch?".into(),
                    text: format!(
                        "It changes {n} file{}: {}{more}. Only the working tree changes; nothing is staged or committed.",
                        if n == 1 { "" } else { "s" },
                        list.join(", ")
                    ),
                    choices: vec![("Apply".into(), RefOp::ApplyPatch(patch, false), false)],
                }));
            }
            Err(e) if patch.is_empty() => self.set_notice(format!("Failed: {e}")),
            Err(e) => self.open_dialog(DialogKind::Ask(Ask {
                title: "The patch does not apply cleanly".into(),
                text: format!(
                    "{e}. A 3-way apply writes conflict markers into those files for you to resolve in Local Changes."
                ),
                choices: vec![("Apply with conflict markers".into(), RefOp::ApplyPatch(patch, true), true)],
            })),
        }
    }

    /// Tests: the Local Changes file menu of the selected row.
    #[cfg(test)]
    pub(super) fn file_menu_at_selected(&mut self) -> Option<Vec<MenuItem>> {
        let side = self.r.changes.side;
        let k = self.r.changes.sel;
        let (r, _) = self.r.changes.hits.iter().find(|h| h.1 == changes_view::CHit::Row(side, k)).copied()?;
        self.file_menu_at(r.x + 4, r.y + r.h / 2)
    }

    /// The menu of the file row under (x, y): Local Changes, the Files
    /// view or a commit's file list.
    fn file_menu_at(&mut self, x: i32, y: i32) -> Option<Vec<MenuItem>> {
        use ffwd_core::tools::DiffOf;
        let root = self.r.git.as_ref()?.root.clone();
        let file_items = |rel: &str, exists: bool| {
            let full = root.join(rel);
            let mut v = Vec::new();
            if exists {
                v.push(item("Open in editor", MenuAct::OpenFile(full.clone(), true)));
                v.push(item("Open with the default app", MenuAct::OpenFile(full.clone(), false)));
                v.push(item(show_label(), MenuAct::ShowFile(full.clone())));
            }
            (v, full)
        };
        if self.r.changes_view {
            let i = self.change_hit_at(x, y)?;
            let Some((_, changes_view::CHit::Row(side, k))) = self.r.changes.hits.get(i).cloned() else { return None };
            self.select_change(side, k);
            let e = self.selected_entry()?;
            let deleted = e.unstaged == b'D' || (side == ffwd_core::changes::Side::Staged && e.staged == b'D');
            let (mut v, full) = file_items(&e.path, !deleted);
            v.push(sep());
            if !e.conflicted && !e.ignored() {
                let staged = side == ffwd_core::changes::Side::Staged;
                v.push(item(if staged { "Unstage" } else { "Stage" }, MenuAct::StageToggle(!staged)));
                if !staged {
                    v.push(danger(
                        if e.untracked() { "Delete the new file…" } else { "Discard changes…" },
                        MenuAct::Discard,
                    ));
                }
                if e.untracked() {
                    v.push(item("Add to .gitignore", MenuAct::Ignore));
                }
                v.push(sep());
            }
            if e.conflicted {
                v.push(item("Resolve in the merge tool", MenuAct::MergeTool(e.path.to_string())));
            } else if !e.untracked() {
                let of = if side == ffwd_core::changes::Side::Staged { DiffOf::Staged } else { DiffOf::Unstaged };
                v.push(item("Open in the diff tool", MenuAct::DiffTool(e.path.to_string(), of)));
            }
            v.push(item("Copy path", MenuAct::Copy(e.path.to_string())));
            v.push(item("Copy full path", MenuAct::Copy(full.to_string_lossy().into_owned())));
            return Some(v);
        }
        if self.r.files_view {
            let path = self.files_path_at(x, y)?;
            let (mut v, full) = file_items(&path, true);
            v.push(sep());
            v.push(item("Copy path", MenuAct::Copy(path)));
            v.push(item("Copy full path", MenuAct::Copy(full.to_string_lossy().into_owned())));
            return Some(v);
        }
        if let Some(Hot::File(k)) = self.hot
            && let Some(d) = &self.r.details
            && let Some(f) = d.files.get(k)
        {
            let sha = self.r.commits.get(self.r.sel).map(|c| c.id.to_string())?;
            let path = f.path.to_string();
            let exists = root.join(&path).exists();
            let (mut v, _) = file_items(&path, exists);
            v.push(sep());
            v.push(item("Open in the diff tool", MenuAct::DiffTool(path.clone(), DiffOf::Commit(sha.clone()))));
            let src = PatchSrc::CommitFile(sha, path.clone());
            v.push(item("Copy as patch", MenuAct::CopyPatch(src.clone())));
            v.push(item("Save as patch…", MenuAct::Name(NameAct::SavePatch { src })));
            v.push(item("Copy path", MenuAct::Copy(path)));
            return Some(v);
        }
        None
    }

    /// Opens a file in the editor (`tools.editor`) or the default app.
    fn open_external(&mut self, p: PathBuf, editor: bool, line: Option<u32>) {
        let cmd = self
            .current_repo_settings()
            .and_then(|r| r.get("tools.editor"))
            .or_else(|| self.settings.get("tools.editor"))
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        match (editor, cmd) {
            (true, Some(c)) => {
                let file = p.to_string_lossy().into_owned();
                if let Some((prog, args)) = ffwd_core::tools::editor_command(&c, &file, line) {
                    std::thread::spawn(move || {
                        let _ = std::process::Command::new(prog).args(args).spawn();
                    });
                }
            }
            (true, None) => {
                self.set_notice("No editor is set (Settings → Tools → Editor); opened with the default app.".into());
                crate::platform::open_path(p);
            }
            (false, _) => crate::platform::open_path(p),
        }
    }

    /// Runs the external diff tool (`Some`) or merge tool (`None`) on a
    /// worker; it returns when the tool closes.
    fn run_tool(&mut self, path: String, diff: Option<ffwd_core::tools::DiffOf>) {
        let (Some(g), Some(t)) = (self.git_for_tab(), self.tabs.get(self.cur_tab)) else { return };
        let sink = (self.sinks)(t.id);
        let merge = diff.is_none();
        self.set_notice(if merge { "Opening the merge tool…".into() } else { "Opening the diff tool…".into() });
        // ffwd's own choice (Settings), else the Git config's (FF-42).
        let key = if merge { "tools.merge_tool" } else { "tools.diff_tool" };
        let tool = self.settings.get(key).map(str::to_string);
        crate::work::spawn("tool", &sink, move |sink| {
            let result = match &diff {
                Some(of) => ffwd_core::tools::difftool(&g, &path, of, tool.as_deref()),
                None => ffwd_core::tools::mergetool(&g, &path, tool.as_deref()),
            };
            sink(Msg::ToolDone { merge, result });
        });
    }

    pub(super) fn on_tool_done(&mut self, merge: bool, r: Result<String, String>) {
        match r {
            Ok(m) => self.set_notice(m),
            Err(e) => self.set_notice(format!("Failed: {e}")),
        }
        if merge && self.r.changes_view {
            self.reload_changes();
        }
    }

    /// Loads the open pull requests when a GitHub remote exists (once per
    /// repository load; a reload after fetch loads them again).
    pub(super) fn load_prs(&mut self) {
        if self.r.prs.is_some() || !ffwd_core::hosting::has_github_remote(&self.r.refs.remotes) {
            return;
        }
        let (Some(g), Some(t)) = (self.git_for_tab(), self.tabs.get(self.cur_tab)) else { return };
        self.r.prs = Some(Err("loading".into()));
        let sink = (self.sinks)(t.id);
        crate::work::spawn("pull-requests", &sink, move |sink| {
            sink(Msg::PullRequests(ffwd_core::hosting::pull_requests(&g)));
            // CI status per commit; nothing to show when it fails.
            if let Ok(runs) = ffwd_core::hosting::ci_runs(&g) {
                sink(Msg::CiRuns(runs));
            }
        });
    }

    /// Opens GitHub's page for a new pull request from the current branch.
    pub(super) fn create_pr(&mut self) {
        let (Some(g), Some(t)) = (self.git_for_tab(), self.tabs.get(self.cur_tab)) else { return };
        let sink = (self.sinks)(t.id);
        self.set_notice("Opening GitHub's new pull request page…".into());
        crate::work::spawn("pr-create", &sink, move |sink| {
            sink(Msg::PrCreate(ffwd_core::hosting::create_web(&g)));
        });
    }

    /// The remote pushes go to: origin, else the first one.
    fn default_remote(&self) -> Option<String> {
        let r = &self.r.refs.remotes;
        r.iter().find(|x| &*x.name == "origin").or(r.first()).map(|x| x.name.to_string())
    }

    pub(super) fn run_refop(&mut self, op: RefOp) {
        // A batch delete or drop uses up the pick (FF-36); a push keeps it.
        if matches!(op, RefOp::DeleteMany(..) | RefOp::DeleteRemoteMany(..) | RefOp::StashDropMany(_)) {
            self.r.side_multi.clear();
            self.damage.rects.push(self.lay.side);
        }
        if let RefOp::Sync(o) = op {
            self.close_dialog();
            return self.start_op(o);
        }
        let (Some(g), Some(t)) = (self.git_for_tab(), self.tabs.get(self.cur_tab)) else { return };
        let sink = (self.sinks)(t.id);
        self.set_notice(format!("{}…", op.verb()));
        crate::work::refop(g, op, &sink);
    }

    pub(super) fn on_refop(&mut self, r: Result<String, OpError>) {
        match r {
            Ok(m) => {
                self.close_dialog();
                self.set_notice(m);
                self.reload_current();
            }
            Err(OpError::NotMerged { branch, tip }) => {
                // Deleting needs force: say what that loses and how to get it back.
                let short = &tip[..tip.len().min(12)];
                self.open_dialog(DialogKind::Ask(Ask {
                    title: format!("{branch} is not merged"),
                    text: format!(
                        "{branch} has commits that are on no other branch. Deleting it anyway hides them; they can be restored with git branch {branch} {short} (also shown afterwards)."
                    ),
                    choices: vec![(format!("Delete {branch} anyway"), RefOp::DeleteBranch(branch.clone(), true), true)],
                }));
            }
            Err(e) => {
                let conflict = matches!(e, OpError::Conflicts { .. });
                let msg = e.to_string();
                match &mut self.dialog {
                    // A name problem: keep the dialog open with the reason.
                    Some(d) if !conflict => {
                        d.busy = false;
                        d.error = Some(msg.clone());
                        let r = self.dialog_rect();
                        self.damage.rects.push(r);
                    }
                    _ => self.close_dialog(),
                }
                self.set_notice(format!("Failed: {msg}"));
                if conflict {
                    // The working tree changed: show it.
                    self.reload_current();
                }
            }
        }
    }

    /// Right mouse button: the menu for the commit row or the sidebar ref
    /// under the pointer.
    pub fn right_click(&mut self, x: i32, y: i32) {
        if self.dialog.is_some() || self.settings_view.is_some() || self.agent_view.is_some() {
            return;
        }
        self.close_menu();
        if self.r.reflog_view && self.reflog_right_click(x, y) {
            return;
        }
        if let Some(i) = self.tab_at(x, y) {
            let items = self.tab_menu(i);
            return self.open_menu(x, y, items);
        }
        let b = self.lay.list_body;
        if !self.r.changes_view && b.contains(x, y) {
            let row = self.r.top + ((y - b.y) / self.lay.row_h.max(1)) as usize;
            if row < self.r.commits.len() {
                self.select(row);
                let items = self.commit_menu(row);
                return self.open_menu(x, y, items);
            }
        }
        if let Some(items) = self.file_menu_at(x, y) {
            return self.open_menu(x, y, items);
        }
        if let Some(items) = self.diff_line_menu(x, y) {
            return self.open_menu(x, y, items);
        }
        if self.lay.side.contains(x, y)
            && let Some(i) = self.side_item_at(x, y)
        {
            let key = self.r.side[i].key.clone();
            self.side_select(&key);
            let items = self.ref_menu(&key);
            self.open_menu(x, y, items);
        }
    }

    /// The diff line under (x, y) in the history's diff (side by side:
    /// the line on the half clicked).
    pub(super) fn diff_line_at(&mut self, x: i32, y: i32) -> Option<usize> {
        if self.r.changes_view || self.r.files_view || self.r.activity_view || self.r.reflog_view {
            return None;
        }
        let d = self.r.diff.clone().filter(|d| !d.binary)?;
        let r = self.diff_rect();
        let body_y = r.y + self.px(32.0);
        if !r.contains(x, y) || y < body_y || self.diff_strip.contains(x, y) {
            return None;
        }
        if !self.diff_split {
            let body = Rect::new(r.x + 1, body_y, r.w - 1, r.bottom() - body_y);
            return self.diff_line_at_y(&d, self.r.diff_top, body, y);
        }
        let row = self.r.diff_top + ((y - body_y) / self.code_lh().max(1)) as usize;
        let (p, _) = self.diff_meta(&d);
        let (l, rr) = *diff_view::split_rows_of(&d, &p).get(row)?;
        if x < r.x + r.w / 2 { l.or(rr) } else { rr.or(l) }
    }

    /// The right-click menu of a diff line: open the file in the editor
    /// at that line, copy the line.
    pub(super) fn diff_line_menu(&mut self, x: i32, y: i32) -> Option<Vec<MenuItem>> {
        // The history's diff, or the Local Changes diff (FF-41).
        let (i, changes) = match self.changes_diff_line_at(x, y) {
            Some(i) => (i, true),
            None => (self.diff_line_at(x, y)?, false),
        };
        let d = if changes { self.r.changes.diff.as_deref()? } else { self.r.diff.as_deref()? };
        let line = d.lines.get(i)?;
        if !matches!(
            line.kind,
            ffwd_core::diff::LineKind::Add | ffwd_core::diff::LineKind::Del | ffwd_core::diff::LineKind::Context
        ) {
            return None;
        }
        let root = self.r.git.as_ref()?.root.clone();
        let full = root.join(&*d.path);
        let mut v = Vec::new();
        if let Some(n) = ffwd_core::diff::new_line_at(d, i) {
            v.push(item(format!("Open in editor at line {n}"), MenuAct::OpenFileAt(full, n)));
        }
        v.push(item("Copy line", MenuAct::Copy(line.text.to_string())));
        Some(v)
    }

    /// The menu of tab `i`.
    pub(super) fn tab_menu(&self, i: usize) -> Vec<MenuItem> {
        let Some(t) = self.tabs.get(i) else { return Vec::new() };
        let p = t.path.clone();
        let mut v = vec![item("Close", MenuAct::CloseTab(i))];
        v.push(item("Rename tab…", MenuAct::Name(NameAct::RenameTab { index: i, old: t.title().to_string() })));
        if self.tabs.len() > 1 {
            v.push(item("Close other tabs", MenuAct::CloseOtherTabs(i)));
        }
        v.push(sep());
        v.push(item(open_folder_label(), MenuAct::OpenFolder(p.clone())));
        v.push(item("Open in terminal", MenuAct::OpenTerminal(p.clone())));
        v.push(item("Copy path", MenuAct::Copy(p.to_string_lossy().into_owned())));
        v.push(sep());
        let cur = t.color;
        for (k, (name, _)) in super::TAB_COLORS.iter().enumerate() {
            let on = cur == Some(k as u8);
            v.push(item(if on { format!("✓ {name}") } else { name.to_string() }, MenuAct::TabColor(i, Some(k as u8))));
        }
        if cur.is_some() {
            v.push(item("No colour", MenuAct::TabColor(i, None)));
        }
        v
    }

    /// `Ctrl+.`: the menu for the selected commit (or sidebar ref).
    pub(super) fn open_menu_for_selection(&mut self) {
        if self.focus == Focus::Side {
            let key = self.r.side_sel.clone();
            let items = self.ref_menu(&key);
            let r = self.lay.side;
            return self.open_menu(r.x + self.px(40.0), r.y + self.px(80.0), items);
        }
        let row = self.r.sel;
        let y = self.row_rect(row).map_or(self.lay.list_body.y, |r| r.bottom());
        let items = self.commit_menu(row);
        self.open_menu(self.lay.list_body.x + self.px(120.0), y, items);
    }

    pub(super) fn paint_menu(&mut self, cv: &mut Canvas) {
        if self.menu.is_none() {
            return;
        }
        let r = self.menu_rect();
        let rects = self.menu_item_rects();
        let Some(m) = self.menu.take() else { return };
        let old = cv.set_clip(cv.clip().intersect(r));
        cv.round_border(r, self.pf(9.0), BORDER_STRONG, BG_FLOAT);
        let ui = self.f.ui;
        // Icons from each item's action (FF-57); a gutter when any has one.
        let icons: Vec<Option<super::Glyph>> =
            m.items.iter().map(|it| it.act.as_ref().and_then(super::icons::for_act)).collect();
        let gutter = if icons.iter().any(Option::is_some) { self.pf(26.0) } else { 0.0 };
        let mut y = r.y + self.px(6.0);
        for (i, it) in m.items.iter().enumerate() {
            let h = self.menu_item_h(it);
            if it.act.is_none() {
                cv.fill(Rect::new(r.x + self.px(12.0), y + h / 2, r.w - self.px(24.0), 1), BORDER);
            } else if let Some(&(_, ir)) = rects.iter().find(|(j, _)| *j == i) {
                let on = m.sel == Some(i) || self.is_hot(Hot::Menu(i));
                if on {
                    cv.fill_round(ir, self.pf(6.0), BG_SELECT);
                }
                let c = if it.danger { DEL_TEXT } else { TEXT };
                if let Some(g) = icons[i] {
                    let ic = if it.danger { DEL_TEXT } else { TEXT_2 };
                    let cy = ir.y as f32 + self.pf(15.0);
                    self.glyph(cv, g, ir.x as f32 + self.pf(19.0), cy, self.pf(7.0), ic);
                }
                self.te.draw(
                    cv,
                    ui,
                    self.pf(12.5),
                    ir.x as f32 + self.pf(10.0) + gutter,
                    (ir.y as f32 + self.pf(19.5)).round(),
                    &it.label,
                    c,
                    ir.w as f32 - self.pf(20.0) - gutter,
                );
            }
            y += h;
        }
        cv.set_clip(old);
        self.menu = Some(m);
    }

    pub(super) fn paint_dialog(&mut self, cv: &mut Canvas) {
        let Some(mut d) = self.dialog.take() else { return };
        let r = self.dialog_rect();
        let old = cv.set_clip(cv.clip().intersect(r));
        cv.round_border(r, self.pf(12.0), BORDER_STRONG, BG_FLOAT);
        let (ui, semi) = (self.f.ui, self.f.semi);
        let pad = self.px(22.0);
        let x0 = (r.x + pad) as f32;
        let inner = (r.w - 2 * pad) as f32;
        let mut hits = Vec::new();
        let (title, text) = match &d.kind {
            DialogKind::Ask(a) => (a.title.clone(), a.text.clone()),
            DialogKind::Name { act, .. } => match act {
                NameAct::CreateBranch { at } => (format!("New branch at {}", short_ref(at)), String::new()),
                NameAct::CreateTag { at } => (format!("New tag at {}", short_ref(at)), String::new()),
                NameAct::Rename { old } => (format!("Rename {old}"), String::new()),
                NameAct::StashSave => (
                    "Stash the working changes".to_string(),
                    "Staged and unstaged changes are saved as a stash and the working tree is reset to HEAD."
                        .to_string(),
                ),
                NameAct::StashRename { name, .. } => (format!("Rename {name}"), String::new()),
                NameAct::AddRemoteUrl => (
                    "Add a remote".to_string(),
                    "The URL or path of the repository (https, ssh or a folder). ffwd fetches it once it is added."
                        .to_string(),
                ),
                NameAct::AddRemoteName { url } => (format!("Name the remote for {url}"), String::new()),
                NameAct::RemoteUrl { name, .. } => (format!("URL of {name}"), String::new()),
                NameAct::RenameRemote { old } => (format!("Rename the remote {old}"), String::new()),
                NameAct::SetUpstream { branch, .. } => (
                    format!("Upstream of {branch}"),
                    "The remote branch it pulls from and pushes to, such as origin/main.".to_string(),
                ),
                NameAct::RenameTab { old, .. } => {
                    (format!("Rename the tab {old}"), "An empty name shows the folder's name again.".to_string())
                }
                NameAct::CloneUrl => (
                    "Clone a repository".to_string(),
                    "Its URL (https or ssh) or a path. ffwd opens it in a new tab when the clone is done.".to_string(),
                ),
                NameAct::CloneDest { url } => (format!("Clone {url} into"), String::new()),
                NameAct::InitDir => (
                    "Create a new repository".to_string(),
                    "The folder (created when missing). An existing repository there is just opened.".to_string(),
                ),
                NameAct::AddSubmoduleUrl => (
                    "Add a submodule".to_string(),
                    "The URL of the repository to include. When it lives next to this one, ffwd offers a relative URL."
                        .to_string(),
                ),
                NameAct::AddSubmodulePath { url } => (format!("Folder for {url}"), String::new()),
                NameAct::SavePatch { .. } => ("Save the patch as".to_string(), String::new()),
                NameAct::ApplyPatchFile => (
                    "Apply a patch file".to_string(),
                    "The patch is checked first; only the working tree changes.".to_string(),
                ),
                NameAct::MoveSubmodule { path } => (
                    format!("Move {path}"),
                    "The new folder, inside the repository. .gitmodules and the index follow.".to_string(),
                ),
            },
        };
        self.te.draw(cv, semi, self.pf(15.5), x0, r.y as f32 + self.pf(36.0), &title, TEXT, inner);
        let mut y = r.y as f32 + self.pf(62.0);
        if !text.is_empty() {
            let size = self.pf(12.5);
            for l in wrap(&mut self.te, ui, size, &text, inner) {
                self.te.draw(cv, ui, self.pf(12.5), x0, y, &l, TEXT_2, inner);
                y += self.pf(18.0);
            }
            if matches!(d.kind, DialogKind::Name { .. }) {
                y += self.pf(10.0);
            }
        }
        if let DialogKind::Name { field, checkout, act } = &d.kind {
            let f = Rect::new(x0 as i32, y as i32 - self.px(12.0), inner as i32, self.px(36.0));
            cv.round_border(f, self.pf(7.0), ACCENT, BG_INPUT);
            let fb = (f.y as f32 + f.h as f32 / 2.0 + self.pf(4.5)).round();
            let tx = f.x as f32 + self.pf(12.0);
            if field.text().is_empty() {
                let ph = match act {
                    NameAct::CreateTag { .. } => "v1.2.0",
                    NameAct::StashSave => "Message (optional)",
                    NameAct::StashRename { .. } => "Stash message",
                    NameAct::AddRemoteUrl | NameAct::RemoteUrl { .. } => "https://github.com/owner/repo.git",
                    NameAct::AddRemoteName { .. } | NameAct::RenameRemote { .. } => "origin",
                    NameAct::SetUpstream { .. } => "origin/main",
                    NameAct::CloneUrl => "https://github.com/owner/repo.git",
                    NameAct::CloneDest { .. } | NameAct::InitDir => "Folder",
                    NameAct::AddSubmoduleUrl => "https://github.com/owner/library.git",
                    NameAct::AddSubmodulePath { .. } | NameAct::MoveSubmodule { .. } => "vendor/library",
                    NameAct::SavePatch { .. } | NameAct::ApplyPatchFile => "File path",
                    _ => "feature/my-change",
                };
                self.te.draw(cv, ui, self.pf(13.0), tx, fb, ph, TEXT_4, inner);
            } else {
                if field.all_selected() {
                    let w = self.te.measure(ui, self.pf(13.0), field.text());
                    cv.fill_round(
                        Rect::new(tx as i32 - 2, f.y + self.px(8.0), w as i32 + 4, f.h - self.px(16.0)),
                        self.pf(3.0),
                        BG_TEXT_SELECT,
                    );
                }
                self.te.draw(cv, ui, self.pf(13.0), tx, fb, field.text(), TEXT, inner);
            }
            let cx = tx + self.te.measure(ui, self.pf(13.0), &field.text()[..field.cursor()]);
            cv.fill(Rect::new(cx as i32 + 1, f.y + self.px(9.0), self.px(1.5).max(1), f.h - self.px(18.0)), ACCENT);
            hits.push((f, DHit::Field));
            y = f.bottom() as f32 + self.pf(24.0);
            if matches!(act, NameAct::CreateBranch { .. } | NameAct::StashSave) {
                let label = match (act, *checkout) {
                    (NameAct::StashSave, true) => "Include untracked files: yes (Tab)",
                    (NameAct::StashSave, false) => "Include untracked files: no (Tab)",
                    (_, true) => "Check it out afterwards: yes (Tab)",
                    (_, false) => "Check it out afterwards: no (Tab)",
                };
                let w = self.te.measure(ui, self.pf(12.5), label);
                let t = Rect::new(
                    x0 as i32 - self.px(4.0),
                    y as i32 - self.px(16.0),
                    w as i32 + self.px(8.0),
                    self.px(24.0),
                );
                if self.is_hot(Hot::Dialog(hits.len())) {
                    cv.fill_round(t, self.pf(5.0), BG_HOVER);
                }
                self.te.draw(cv, ui, self.pf(12.5), x0, y, label, TEXT_2, inner);
                hits.push((t, DHit::Toggle));
            }
        }
        if let Some(e) = &d.error {
            self.te.draw(cv, ui, self.pf(12.0), x0, (r.bottom() - self.px(58.0)) as f32, e, DEL, inner);
        }
        // Buttons, right to left: Cancel, then the choices.
        let choices: Vec<(String, bool)> = match &d.kind {
            DialogKind::Ask(a) => a.choices.iter().map(|c| (c.0.clone(), c.2)).collect(),
            DialogKind::Name { act, .. } => vec![(
                match act {
                    NameAct::CreateBranch { .. } => "Create branch".into(),
                    NameAct::CreateTag { .. } => "Create tag".into(),
                    NameAct::Rename { .. } | NameAct::StashRename { .. } => "Rename".into(),
                    NameAct::StashSave => "Stash".into(),
                    NameAct::AddRemoteUrl => "Next".into(),
                    NameAct::AddRemoteName { .. } => "Add and fetch".into(),
                    NameAct::RemoteUrl { .. } => "Save URL".into(),
                    NameAct::RenameRemote { .. } | NameAct::RenameTab { .. } => "Rename".into(),
                    NameAct::SetUpstream { .. } => "Track".into(),
                    NameAct::CloneUrl => "Next".into(),
                    NameAct::CloneDest { .. } => "Clone".into(),
                    NameAct::InitDir => "Create".into(),
                    NameAct::AddSubmoduleUrl => "Next".into(),
                    NameAct::AddSubmodulePath { .. } => "Add submodule".into(),
                    NameAct::MoveSubmodule { .. } => "Move".into(),
                    NameAct::SavePatch { .. } => "Save".into(),
                    NameAct::ApplyPatchFile => "Check".into(),
                },
                false,
            )],
        };
        let by = r.bottom() - self.px(48.0);
        let bh = self.px(32.0);
        let mut bx = r.right() - pad;
        let cw = (self.te.measure(semi, self.pf(12.5), "Cancel") + self.pf(28.0)) as i32;
        let cancel = Rect::new(bx - cw, by, cw, bh);
        let (bg, border) = self.control_colors(Hot::Dialog(hits.len()), BG_BAR, BORDER);
        cv.round_border(cancel, self.pf(7.0), border, bg);
        self.te.draw(
            cv,
            semi,
            self.pf(12.5),
            cancel.x as f32 + self.pf(14.0),
            (by as f32 + self.pf(21.0)).round(),
            "Cancel",
            TEXT_2,
            cw as f32,
        );
        hits.push((cancel, DHit::Cancel));
        bx = cancel.x - self.px(8.0);
        for (i, (label, dangerous)) in choices.iter().enumerate().rev() {
            let label = if d.busy && i == d.sel { "Working…".to_string() } else { label.clone() };
            let w = (self.te.measure(semi, self.pf(12.5), &label) + self.pf(28.0)) as i32;
            let b = Rect::new(bx - w, by, w, bh);
            let selected = i == d.sel;
            let hot = self.is_hot(Hot::Dialog(hits.len()));
            let (bg, border, fg) = match (*dangerous, selected || hot) {
                (true, true) => (DEL_BG, DEL, DEL_TEXT),
                (true, false) => (BG_BAR, DANGER_BORDER, DEL_TEXT),
                (false, true) => (ACCENT, ACCENT, ACCENT_TEXT),
                (false, false) => (BG_BAR, BORDER, TEXT),
            };
            cv.round_border(b, self.pf(7.0), border, bg);
            self.te.draw(
                cv,
                semi,
                self.pf(12.5),
                b.x as f32 + self.pf(14.0),
                (by as f32 + self.pf(21.0)).round(),
                &label,
                fg,
                w as f32,
            );
            hits.push((b, DHit::Choice(i)));
            bx = b.x - self.px(8.0);
        }
        let hint = "Enter runs the highlighted button · Esc cancels";
        self.te.draw(
            cv,
            ui,
            self.pf(11.0),
            x0,
            (by as f32 + self.pf(21.0)).round(),
            hint,
            TEXT_4,
            (bx - x0 as i32) as f32 - self.pf(8.0),
        );
        cv.set_clip(old);
        d.hits = hits;
        self.dialog = Some(d);
    }

    /// Snapshot only: the selected commit's menu, first item highlighted.
    pub fn snapshot_menu(&mut self) {
        self.open_menu_for_selection();
        if let Some(m) = &mut self.menu {
            m.sel = m.items.iter().position(|i| i.act.is_some());
        }
    }

    /// Snapshot only: sidebar items picked for a batch (FF-36), with the
    /// batch menu open at the first one.
    pub fn snapshot_pick(&mut self, keys: &str) {
        let keys: Vec<String> = keys.split(';').map(str::trim).filter(|k| !k.is_empty()).map(String::from).collect();
        self.r.side_multi.extend(keys.iter().cloned());
        let Some(first) = keys.first() else { return };
        let Some(i) = self.r.side.iter().position(|it| &it.key == first) else { return };
        let l = self.side_list_rect();
        let y = l.y + (i.saturating_sub(self.r.side_top)) as i32 * self.side_row_h() + self.side_row_h() / 2;
        let items = self.ref_menu(first);
        self.open_menu(l.x + l.w / 2, y, items);
    }

    /// Snapshot only: the "new branch" prompt with a typed name.
    pub fn snapshot_prompt(&mut self) {
        let at = self.r.commits.get(self.r.sel).map(|c| c.id.to_string()).unwrap_or_default();
        self.open_dialog(DialogKind::Name {
            act: NameAct::CreateBranch { at },
            field: TextField::new("feature/blame-view"),
            checkout: true,
        });
    }

    pub(super) fn menu_hit_at(&self, x: i32, y: i32) -> Option<usize> {
        self.menu_item_rects().into_iter().find(|(_, r)| r.contains(x, y)).map(|(i, _)| i)
    }

    pub(super) fn dialog_hit_at(&self, x: i32, y: i32) -> Option<usize> {
        self.dialog.as_ref()?.hits.iter().position(|h| h.0.contains(x, y))
    }
}

/// A SHA shortened; branch and tag names as they are.
fn short_ref(s: &str) -> String {
    if s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit()) { s[..8].to_string() } else { s.to_string() }
}

fn open_folder_label() -> &'static str {
    if cfg!(windows) {
        "Show in Explorer"
    } else if cfg!(target_os = "macos") {
        "Show in Finder"
    } else {
        "Open the folder"
    }
}

fn show_label() -> &'static str {
    if cfg!(windows) {
        "Show in Explorer"
    } else if cfg!(target_os = "macos") {
        "Show in Finder"
    } else {
        "Show in the folder"
    }
}

/// The actions on a reflog entry's commit (FF-43).
pub(super) fn reflog_items(ui: &Ui, sha: &str, in_history: bool) -> Vec<MenuItem> {
    let short = sha[..sha.len().min(8)].to_string();
    let mut v = vec![item("Create branch here…", MenuAct::Name(NameAct::CreateBranch { at: sha.to_string() }))];
    if in_history {
        v.push(item("Show in history", MenuAct::ShowCommit(sha.to_string())));
    }
    v.push(item(format!("Check out {short} (detached)"), MenuAct::Run(RefOp::Checkout(sha.to_string()))));
    if let Some(h) = ui.r.refs.head_branch.as_deref() {
        v.push(sep());
        v.push(danger(format!("Reset {h} to {short}…"), MenuAct::Ask(Box::new(ui.reset_ask(sha, &short, h)))));
    }
    v.push(sep());
    v.push(item("Copy SHA", MenuAct::Copy(sha.to_string())));
    v
}

/// What a patch is made from (FF-46).
#[derive(Clone, Debug)]
pub enum PatchSrc {
    Commit(String),
    /// One file of a commit: (sha, path).
    CommitFile(String, String),
    Stash(String),
    /// The staged (true) or unstaged changes.
    Working(bool),
}
