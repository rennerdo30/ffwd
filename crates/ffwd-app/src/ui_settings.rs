//! The settings page (gear button, Ctrl+,): the whole window below the top
//! bar, with sections on the left and each section's settings as a card
//! (FF-57). On/off settings are switches, long lists are dropdowns, and
//! only two or three short choices are segmented. A search field finds any
//! setting. Agent settings apply to all repositories or to the open one;
//! every overridden row says so and can be reset.
//!
//! Every change is saved to `settings.toml` at once, on a worker; the file
//! stays hand-editable and keeps its comments.

use super::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Row {
    /// System, dark or light (FF-57).
    Theme,
    /// Where the views sit: rail, tabs, list or icon bar (FF-57).
    Sidebar,
    Layout,
    Updates,
    Channel,
    /// External merge and diff tools (FF-42).
    MergeTool,
    DiffTool,
    /// Fetch automatically (FF-39).
    AutoFetch,
    Roots,
    /// Git's own settings (git config, FF-57): identity, everywhere and
    /// in this repository; the version; credentials and certificates.
    GitName,
    GitEmail,
    RepoGitName,
    RepoGitEmail,
    GitVersion,
    CredHelper,
    SslVerify,
    SslCaInfo,
    SslBackend,
    AiEndpoint,
    AiModel,
    Editor,
    GitPath,
    AiKeyEnv,
    OpenFile,
    CheckUpdates,
    // Agents (experimental).
    AgentsOn,
    Preset,
    AgentCommand,
    Sandbox,
    /// The WSL distribution the sandbox uses (Windows).
    WslDistro,
    AllowUnsandboxed,
    Network,
    GitAccess,
    HomeAccess,
    /// One path the agent may use (FF-23): the preset's own, or added.
    Grant(u8),
    /// Add a folder or file to the access list.
    AddGrant,
    Setup,
    After,
    OnDone,
    Timeout,
    AgentsFile,
    /// About: the version, the licences and the source.
    AboutVersion,
    AboutLicense,
    AboutNotices,
    AboutSource,
}

/// One section: its place in the sidebar, its heading and its rows.
struct Section {
    nav: &'static str,
    about: &'static str,
    glyph: Glyph,
    /// Agent settings: they can differ per repository.
    agents: bool,
    rows: &'static [Row],
}

const SECTIONS: &[Section] = &[
    Section {
        nav: "Appearance",
        about: "How ffwd looks: its theme, and where the commit details go.",
        glyph: Glyph::Expand,
        agents: false,
        rows: &[Row::Theme, Row::Sidebar, Row::Layout],
    },
    Section {
        nav: "Updates",
        about: "ffwd updates itself from its releases; installed copies check once a day.",
        glyph: Glyph::Update,
        agents: false,
        rows: &[Row::Updates, Row::Channel, Row::CheckUpdates],
    },
    Section {
        nav: "Repositories",
        about: "Where Open repository looks for repositories.",
        glyph: Glyph::Folder,
        agents: false,
        rows: &[Row::Roots],
    },
    Section {
        nav: "Git",
        about: "Who you are in commits, how Git signs in, and which certificates it trusts. These are Git's own settings (git config): other Git tools use them too.",
        glyph: Glyph::Commit,
        agents: false,
        rows: &[
            Row::GitName,
            Row::GitEmail,
            Row::RepoGitName,
            Row::RepoGitEmail,
            Row::GitPath,
            Row::GitVersion,
            Row::CredHelper,
            Row::SslVerify,
            Row::SslCaInfo,
            Row::SslBackend,
        ],
    },
    Section {
        nav: "Tools",
        about: "The programs ffwd starts: your editor, and the merge and diff tools.",
        glyph: Glyph::Editor,
        agents: false,
        rows: &[Row::Editor, Row::MergeTool, Row::DiffTool],
    },
    Section {
        nav: "Remotes",
        about: "Fetching from your remotes in the background.",
        glyph: Glyph::Fetch,
        agents: false,
        rows: &[Row::AutoFetch],
    },
    Section {
        nav: "AI",
        about: "Commit messages from a model you choose. Off until an endpoint is set; local servers first. ffwd shows every request before it is sent.",
        glyph: Glyph::Text,
        agents: false,
        rows: &[Row::AiEndpoint, Row::AiModel, Row::AiKeyEnv],
    },
    Section {
        nav: "Advanced",
        about: "Every setting as text, including the ones without a control here.",
        glyph: Glyph::Gear,
        agents: false,
        rows: &[Row::OpenFile, Row::AgentsFile],
    },
    Section {
        nav: "About",
        about: "ffwd (say fast-forward): a fast Git client, free and open source under the MIT licence.",
        glyph: Glyph::Head,
        agents: false,
        rows: &[Row::AboutVersion, Row::AboutLicense, Row::AboutNotices, Row::AboutSource],
    },
    Section {
        nav: "Agent",
        about: "A coding agent works on a task in its own worktree and branch, inside a sandbox. You review and merge its commits.",
        glyph: Glyph::Terminal,
        agents: true,
        rows: &[Row::AgentsOn, Row::Preset, Row::AgentCommand],
    },
    Section {
        nav: "Sandbox",
        about: "What the agent can reach while it runs. The sandbox keeps it inside its worktree.",
        glyph: Glyph::Ignore,
        agents: true,
        rows: &[Row::Sandbox, Row::WslDistro, Row::AllowUnsandboxed, Row::Network, Row::GitAccess],
    },
    Section {
        nav: "Access",
        about: "The folders and files the agent may use besides its worktree. Your home stays private unless you choose otherwise.",
        glyph: Glyph::Link,
        agents: true,
        rows: &[Row::HomeAccess, Row::AddGrant],
    },
    Section {
        nav: "Scripts & limits",
        about: "Commands around each run, and how long a run may take.",
        glyph: Glyph::Plan,
        agents: true,
        rows: &[Row::Setup, Row::After, Row::OnDone, Row::Timeout],
    },
];

/// The first agent section (the agents panel opens Settings there).
const FIRST_AGENT_SECTION: usize = 9;

/// The sidebar groups: (title, first section, end).
const GROUPS: [(&str, usize, usize); 2] = [("General", 0, FIRST_AGENT_SECTION), ("Agents", FIRST_AGENT_SECTION, 13)];

/// How a row is edited.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ctl {
    /// A value to read, not to change.
    Info,
    Toggle,
    Segmented,
    Dropdown,
    Text,
    Button,
}

impl Row {
    fn label(self) -> &'static str {
        match self {
            Row::Theme => "Theme",
            Row::Sidebar => "Views",
            Row::Layout => "Commit details",
            Row::Updates => "Automatic updates",
            Row::AutoFetch => "Fetch automatically",
            Row::Channel => "Update channel",
            Row::Roots => "Repository folders",
            Row::GitName => "Name",
            Row::GitEmail => "Email",
            Row::RepoGitName => "Name in this repository",
            Row::RepoGitEmail => "Email in this repository",
            Row::GitVersion => "Git version",
            Row::CredHelper => "Credential helper",
            Row::SslVerify => "Verify certificates",
            Row::SslCaInfo => "Certificate authorities",
            Row::SslBackend => "TLS backend",
            Row::AiEndpoint => "Endpoint",
            Row::AiModel => "Model",
            Row::Editor => "Editor",
            Row::MergeTool => "Merge tool",
            Row::DiffTool => "Diff tool",
            Row::GitPath => "Git program",
            Row::AiKeyEnv => "API key variable",
            Row::OpenFile => "Settings file",
            Row::CheckUpdates => "Check now",
            Row::AgentsOn => "Agents",
            Row::Preset => "Agent",
            Row::AgentCommand => "Command",
            Row::Sandbox => "Sandbox",
            Row::WslDistro => "WSL distribution",
            Row::AllowUnsandboxed => "Allow running without a sandbox",
            Row::Network => "Network",
            Row::GitAccess => "Git access",
            Row::HomeAccess => "Home folder",
            Row::Grant(_) => "",
            Row::AddGrant => "Add a folder or file",
            Row::Setup => "Setup script",
            Row::After => "After script",
            Row::OnDone => "When a run ends",
            Row::Timeout => "Time limit (minutes)",
            Row::AgentsFile => "All agent settings",
            Row::AboutVersion => "Version",
            Row::AboutLicense => "Licence",
            Row::AboutNotices => "Third-party licences",
            Row::AboutSource => "Source code",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Row::Theme => "System follows your computer's light or dark mode, also when it changes.",
            Row::Sidebar => {
                "How Changes, History, Files, Activity, Reflog and Agents are offered: a rail of icons at the left edge, tabs over the content, a list in the sidebar or a bar of icons."
            }
            Row::Layout => "Where the selected commit's details and diff go.",
            Row::Updates => "Check for new versions once a day and offer to install them.",
            Row::AutoFetch => {
                "Fetch every remote of the open repository. Quiet unless something new arrives; never asks for a password."
            }
            Row::Channel => "Automatic follows the running version: a beta gets betas.",
            Row::Roots => {
                "Folders searched for repositories, separated by ;. Empty: the folders next to the open repositories."
            }
            Row::GitName => "Your name in new commits, in every repository (git config --global user.name).",
            Row::GitEmail => "Your email in new commits. On GitHub, your noreply address keeps your email private.",
            Row::RepoGitName => "Only in this repository, for example a work identity. Empty: the name above.",
            Row::RepoGitEmail => "Only in this repository. Empty: the email above.",
            Row::GitVersion => "The Git program ffwd runs.",
            Row::CredHelper => "How Git keeps passwords and tokens, for example manager. Empty: Git's default.",
            Row::SslVerify => {
                "Check each server's certificate. Turn it off only for a server you trust on a private network."
            }
            Row::SslCaInfo => "Extra certificate authorities in a .pem file, for example a company server's own.",
            Row::SslBackend => "Which TLS library Git uses. On Windows, schannel uses the Windows certificate store.",
            Row::AiEndpoint => "An OpenAI-compatible URL, for example a local Ollama, LM Studio or llama.cpp server.",
            Row::AiModel => "The model name the endpoint expects.",
            Row::Editor => {
                "Opens files from the file menus: a command such as code, or a full path. Empty: the default app."
            }
            Row::MergeTool => "The tools Git can start on this PC. Git config uses your merge.tool.",
            Row::DiffTool => "The tools Git can start on this PC. Git config uses your diff.tool.",
            Row::GitPath => "The git program (or FFWD_GIT). Empty: git from PATH. Applies at the next start.",
            Row::AiKeyEnv => "The name of an environment variable holding the API key. The key itself is never stored.",
            Row::OpenFile => "settings.toml in your editor. ffwd keeps your comments when it changes a value.",
            Row::CheckUpdates => "Look for a new version right away.",
            Row::AgentsOn => "Experimental. Off: no agent runs, and the agents view stays empty.",
            Row::Preset => {
                "A built-in tool brings its command and the folders it needs. Custom: your own command below."
            }
            Row::AgentCommand => {
                "Empty uses the agent's built-in command. Placeholders: {prompt} {prompt_file} {worktree} {branch}."
            }
            Row::Sandbox => "Automatic: bubblewrap on Linux, WSL + bubblewrap on Windows, the macOS sandbox on macOS.",
            Row::WslDistro => "The Linux distribution the agent runs in. Default: WSL's default distribution.",
            Row::AllowUnsandboxed => "Needed for sandbox None, where the agent has all your permissions.",
            Row::Network => "Agents that call a hosted model need it.",
            Row::GitAccess => {
                "Read-only: the agent edits files and ffwd commits them. Writable: the agent commits itself."
            }
            Row::HomeAccess => "Private: an empty home folder, plus only the paths below.",
            Row::Grant(_) => "",
            Row::AddGrant => "Added read-only; switch it to Write after. ~/… is the agent's home.",
            Row::Setup => {
                "Runs in the worktree before the agent, for example npm ci. If it fails, the agent does not start."
            }
            Row::After => "Runs after the agent and before ffwd commits, for example the tests.",
            Row::OnDone => "Runs on your machine when a run ends, for example a notification.",
            Row::Timeout => "The agent is stopped after this, from 1 to 1440 minutes.",
            Row::AgentsFile => "Environment, folders, branch prefix, prompt template, shells.",
            Row::AboutVersion => "The version you are running.",
            Row::AboutLicense => "MIT: use, change and share ffwd freely; keep the copyright notice.",
            Row::AboutNotices => "The open-source libraries ffwd is built from, each with its licence text.",
            Row::AboutSource => "The repository: code, releases and issues.",
        }
    }

    fn key(self) -> Option<&'static str> {
        match self {
            Row::Roots => Some("repos.roots"),
            Row::GitName => Some("git:global:user.name"),
            Row::GitEmail => Some("git:global:user.email"),
            Row::RepoGitName => Some("git:local:user.name"),
            Row::RepoGitEmail => Some("git:local:user.email"),
            Row::CredHelper => Some("git:global:credential.helper"),
            Row::SslCaInfo => Some("git:global:http.sslCAInfo"),
            Row::AiEndpoint => Some("ai.endpoint"),
            Row::AiModel => Some("ai.model"),
            Row::Editor => Some("tools.editor"),
            Row::GitPath => Some("git.path"),
            Row::AiKeyEnv => Some("ai.key_env"),
            Row::AgentCommand => Some("agents.command"),
            // A text row; its Enter adds to the list (see commit_edit).
            Row::AddGrant => Some(ADD_PATH_KEY),
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
            Row::WslDistro => "agents.wsl_distro",
            Row::AllowUnsandboxed => "agents.allow_unsandboxed",
            Row::Network => "agents.network",
            Row::GitAccess => "agents.git_access",
            Row::HomeAccess => "agents.home_access",
            Row::AddGrant | Row::Grant(_) => return None,
            r => return r.key().filter(|k| k.starts_with("agents.")),
        })
    }

    fn placeholder(self) -> &'static str {
        match self {
            Row::Roots => "Folders next to the open repositories",
            Row::GitName => "Your Name",
            Row::GitEmail => "you@example.com",
            Row::CredHelper => "Git's default · e.g. manager",
            Row::SslCaInfo => "None",
            Row::AiEndpoint => "Off · e.g. http://localhost:11434/v1",
            Row::AiModel => "e.g. qwen2.5-coder:14b",
            Row::Editor => "Default app · e.g. code",
            Row::GitPath => "git from PATH",
            Row::AiKeyEnv => "None (local servers) · e.g. OPENAI_API_KEY",
            Row::AgentCommand => "e.g. aider --yes-always --message-file {prompt_file}",
            Row::AddGrant => "~/.gitconfig, /data/docs, …",
            Row::Setup | Row::After | Row::OnDone => "None",
            Row::Timeout => "60",
            _ => "",
        }
    }

    fn choices(self) -> &'static [&'static str] {
        match self {
            Row::Theme => &["System", "Dark", "Light"],
            Row::Sidebar => &["Rail", "Tabs", "List", "Icon bar"],
            Row::Layout => &["Below the history", "Beside the history"],
            Row::Updates => &["On", "Off"],
            Row::AutoFetch => &["Off", "5 min", "10 min", "30 min"],
            Row::SslVerify => &["On", "Off"],
            Row::SslBackend => &["Git's default", "openssl", "schannel"],
            Row::Channel => &["Automatic", "Stable", "Beta"],
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

    fn ctl(self) -> Ctl {
        match self {
            Row::GitVersion | Row::AboutVersion => Ctl::Info,
            Row::Updates | Row::AgentsOn | Row::AllowUnsandboxed | Row::Network | Row::SslVerify => Ctl::Toggle,
            Row::Preset
            | Row::Sandbox
            | Row::WslDistro
            | Row::MergeTool
            | Row::DiffTool
            | Row::AutoFetch
            | Row::SslBackend => Ctl::Dropdown,
            Row::OpenFile
            | Row::CheckUpdates
            | Row::AgentsFile
            | Row::AboutLicense
            | Row::AboutNotices
            | Row::AboutSource => Ctl::Button,
            r if r.key().is_some() => Ctl::Text,
            _ => Ctl::Segmented,
        }
    }

    /// The choice index that means "on" for a switch.
    fn on_index(self) -> usize {
        match self {
            Row::Updates | Row::Network | Row::SslVerify => 0,
            _ => 1,
        }
    }
}

/// Path rows with Browse…: Some(true) a folder, Some(false) a file.
fn browse_kind(row: Row) -> Option<bool> {
    match row {
        Row::Roots | Row::AddGrant => Some(true),
        Row::Editor | Row::GitPath | Row::SslCaInfo => Some(false),
        _ => None,
    }
}

/// The pseudo-key of the "Add a folder or file" row (never saved).
const ADD_PATH_KEY: &str = "agents.__add_path";

/// One entry of the agent's access list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Grant {
    pub path: String,
    /// 0 off (preset) or remove (added), 1 read, 2 write.
    pub level: u8,
    /// The tool whose preset brings it, or None when the user added it.
    pub tool: Option<&'static str>,
}

/// `a; b ;c` as a list.
fn split_paths(s: &str) -> Vec<String> {
    s.split(';').map(str::trim).filter(|p| !p.is_empty()).map(str::to_string).collect()
}

/// What a click on the settings page hits.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum SHit {
    Nav(usize),
    Search,
    Scope,
    Close,
    /// A row (its index in the visible rows) and, for segmented rows, the
    /// choice.
    Seg(usize, usize),
    Toggle(usize),
    Drop(usize),
    Field(usize),
    Button(usize),
    /// Remove this repository's value for the row.
    Reset(usize),
    /// The row is overridden in this repository: edit it there.
    EditThere,
    /// The status box's Turn on.
    TurnOn,
    /// The system's picker for a path row (FF-57).
    Browse(usize),
}

pub struct SettingsView {
    section: usize,
    /// Content scrolled up, in pixels.
    scroll: i32,
    pub(super) sel: usize,
    /// The text row being edited and its field.
    edit: Option<(usize, TextField)>,
    search: TextField,
    search_focus: bool,
    /// The keyboard moved the selection: it shows.
    keys: bool,
    /// Click targets from the last paint.
    pub(super) hits: Vec<(Rect, SHit)>,
    /// Each visible row's place in the content (y from the content top,
    /// height), from the last layout.
    places: Vec<(i32, i32)>,
    content_h: i32,
}

const PRESET_IDS: [&str; 6] = ["custom", "claude", "codex", "antigravity", "opencode", "muse"];

impl Ui {
    pub fn open_settings(&mut self) {
        // From the agents panel, Settings opens on the agent settings.
        let page = if self.agent_view.take().is_some() { 1 } else { 0 };
        self.open_settings_page(page);
    }

    /// A row's choices: fixed, or found on this PC (WSL distributions,
    /// merge and diff tools).
    pub(super) fn row_choices(&self, row: Row) -> Vec<String> {
        if let Row::Grant(k) = row {
            let preset = self.grants().get(k as usize).is_some_and(|g| g.tool.is_some());
            let c: &[&str] = if preset { &["Off", "Read", "Write"] } else { &["Remove", "Read", "Write"] };
            return c.iter().map(|s| s.to_string()).collect();
        }
        let Some((key, first, list)) = self.list_row(row) else {
            return row.choices().iter().map(|s| s.to_string()).collect();
        };
        let mut v = vec![first.to_string()];
        // WSL's own helper distributions are not for agents.
        v.extend(list.iter().filter(|d| !(row == Row::WslDistro && d.starts_with("docker-desktop"))).cloned());
        if let Some(cur) = self.sget(key).filter(|c| !c.is_empty())
            && !v.iter().any(|d| d == cur)
        {
            v.push(cur.to_string());
        }
        v
    }

    /// A choice as a dropdown shows it.
    fn choice_label(&self, row: Row, i: usize, v: &str) -> String {
        match row {
            Row::Preset if i == 0 => "Custom command".into(),
            Row::Preset => {
                ffwd_agent::presets::find(PRESET_IDS[i.min(5)]).map_or(v.to_string(), |p| p.name.to_string())
            }
            Row::Sandbox => [
                "Automatic",
                "bubblewrap (Linux)",
                "WSL + bubblewrap (Windows)",
                "macOS sandbox",
                "Custom wrapper",
                "None: all your permissions",
            ][i.min(5)]
            .into(),
            Row::AutoFetch => ["Off", "Every 5 minutes", "Every 10 minutes", "Every 30 minutes"][i.min(3)].into(),
            Row::WslDistro if i == 0 => "WSL's default".into(),
            _ => v.to_string(),
        }
    }

    /// Rows whose choices are found on this PC: (setting, the "use the
    /// default" choice, the found ones).
    fn list_row(&self, row: Row) -> Option<(&'static str, &'static str, &[String])> {
        Some(match row {
            Row::WslDistro => ("agents.wsl_distro", "Default", &self.wsl_distros),
            Row::MergeTool => ("tools.merge_tool", "Git config", &self.merge_tools),
            Row::DiffTool => ("tools.diff_tool", "Git config", &self.diff_tools),
            _ => return None,
        })
    }

    /// Opens Settings: page 0 the general settings, 1 the agent settings.
    pub fn open_settings_page(&mut self, page: usize) {
        self.open_settings_section(if page == 1 { FIRST_AGENT_SECTION } else { 0 });
    }

    /// Opens Settings on the section named `nav` (snapshots, links).
    pub fn open_settings_named(&mut self, nav: &str) {
        let i = SECTIONS.iter().position(|s| s.nav.eq_ignore_ascii_case(nav)).unwrap_or(0);
        self.open_settings_section(i);
    }

    pub(super) fn open_settings_section(&mut self, section: usize) {
        // The WSL distributions, for the agents' sandbox (never in tests).
        if cfg!(windows) && !cfg!(test) {
            let sink = (self.sinks)(0);
            std::thread::spawn(move || sink(Msg::WslDistros(crate::platform::wsl_distros())));
        }
        // The merge and diff tools Git can start (never in tests).
        if !cfg!(test) {
            let sink = (self.sinks)(0);
            let git = self.r.git.clone().unwrap_or(ffwd_core::git::Git { root: std::env::temp_dir() });
            std::thread::spawn(move || {
                let merge = ffwd_core::tools::available(&git, true);
                let diff = ffwd_core::tools::available(&git, false);
                sink(Msg::Tools(merge, diff));
            });
        }
        self.load_git_conf();
        self.palette = None;
        self.damage.full = true;
        self.settings_view = Some(SettingsView {
            section: section.min(SECTIONS.len() - 1),
            scroll: 0,
            sel: 0,
            edit: None,
            search: TextField::default(),
            search_focus: false,
            keys: false,
            hits: Vec::new(),
            places: Vec::new(),
            content_h: 0,
        });
        self.agent_scope_repo = false;
    }

    pub(super) fn close_settings(&mut self) {
        if self.settings_view.take().is_some() {
            self.damage.full = true;
        }
    }

    /// The settings element under the pointer (for hover): its hit index.
    pub(super) fn settings_hit_at(&self, x: i32, y: i32) -> Option<Hot> {
        let v = self.settings_view.as_ref()?;
        v.hits.iter().position(|h| h.0.contains(x, y)).map(|i| Hot::Setting(i, 0))
    }

    /// Is hit `i` a text field (for the I-beam pointer)?
    pub(super) fn setting_is_text(&self, i: usize) -> bool {
        self.settings_view.as_ref().is_some_and(|v| matches!(v.hits.get(i), Some((_, SHit::Field(_) | SHit::Search))))
    }

    /// A row's label; an access row shows its path.
    fn row_label(&self, row: Row) -> String {
        match row {
            Row::Grant(k) => self.grants().get(k as usize).map_or(String::new(), |g| g.path.clone()),
            // The repository by name: "this repository" makes you look.
            Row::RepoGitName => format!("Name in {}", self.r.repo_name),
            Row::RepoGitEmail => format!("Email in {}", self.r.repo_name),
            r => r.label().to_string(),
        }
    }

    /// What an empty field means; for this repository's identity, the
    /// global value that applies instead.
    fn row_placeholder(&self, row: Row) -> String {
        let global = match row {
            Row::RepoGitName => self.sget("git:global:user.name"),
            Row::RepoGitEmail => self.sget("git:global:user.email"),
            _ => return row.placeholder().to_string(),
        };
        match global {
            Some(v) => format!("{v} (from all repositories)"),
            None => "Not set anywhere yet".into(),
        }
    }

    pub(super) fn row_hint(&self, row: Row) -> String {
        match row {
            Row::Grant(k) => match self.grants().get(k as usize).and_then(|g| g.tool) {
                Some(tool) => format!("{tool} keeps its login and settings here."),
                None => "Added by you.".into(),
            },
            Row::RepoGitName | Row::RepoGitEmail => {
                let what = if row == Row::RepoGitName { "name" } else { "email" };
                format!(
                    "Only for commits in {}, for example a work identity. Empty: your {what} for all repositories.",
                    self.r.repo_name
                )
            }
            // The last check's answer, once there is one.
            Row::CheckUpdates if !self.update_line.is_empty() => self.update_line.clone(),
            Row::AboutVersion => {
                let os = match std::env::consts::OS {
                    "windows" => "Windows",
                    "macos" => "macOS",
                    "linux" => "Linux",
                    o => o,
                };
                format!("{os}, {}. {}", std::env::consts::ARCH, Row::AboutVersion.hint())
            }
            r => r.hint().to_string(),
        }
    }

    /// Opens a file that ships next to ffwd (installer and archives). A
    /// build without it: the licence from inside ffwd; the notices, a word
    /// on where they come from.
    fn open_beside_exe(&mut self, name: &'static str) {
        if cfg!(test) {
            return;
        }
        let sink = (self.sinks)(0);
        std::thread::spawn(move || {
            let beside = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join(name)));
            match beside.filter(|p| p.is_file()) {
                Some(p) => open_target(p.to_string_lossy().into_owned()),
                None if name == "LICENSE.txt" => {
                    let p = std::env::temp_dir().join("ffwd-LICENSE.txt");
                    match std::fs::write(&p, LICENSE) {
                        Ok(()) => open_target(p.to_string_lossy().into_owned()),
                        Err(e) => sink(Msg::Notice(format!("Could not show the licence: {e}"))),
                    }
                }
                None => sink(Msg::Notice(format!(
                    "{name} comes with installed copies of ffwd, next to the program; this build has none."
                ))),
            }
        });
    }

    /// The rows on screen with their section: the current section's, or
    /// with a search, every matching row.
    fn visible(&self) -> Vec<(usize, Row)> {
        let Some(v) = &self.settings_view else { return Vec::new() };
        let q = v.search.text().trim().to_lowercase();
        let mut out = Vec::new();
        for (si, s) in SECTIONS.iter().enumerate() {
            if q.is_empty() && si != v.section {
                continue;
            }
            let section_hit = s.nav.to_lowercase().contains(&q);
            for &r in s.rows {
                let mut rows = Vec::new();
                if r == Row::AddGrant {
                    rows.extend((0..self.grants().len().min(64)).map(|k| Row::Grant(k as u8)));
                }
                rows.push(r);
                for r in rows {
                    if matches!(r, Row::RepoGitName | Row::RepoGitEmail) && self.r.git.is_none() {
                        continue;
                    }
                    let hit = q.is_empty()
                        || section_hit
                        || self.row_label(r).to_lowercase().contains(&q)
                        || self.row_hint(r).to_lowercase().contains(&q);
                    if hit {
                        out.push((si, r));
                    }
                }
            }
        }
        out
    }

    /// The visible rows.
    pub(super) fn rows(&self) -> Vec<Row> {
        self.visible().into_iter().map(|(_, r)| r).collect()
    }

    /// The agent's access list: the preset's own paths (with their level),
    /// then the paths added by the user (FF-23).
    pub(super) fn grants(&self) -> Vec<Grant> {
        let mut v = Vec::new();
        let access = ffwd_agent::config::parse_preset_access(self.sget("agents.preset_access").unwrap_or(""));
        if let Some(p) = self.sget("agents.preset").and_then(ffwd_agent::presets::find) {
            for h in p.home_write {
                let level = match access.iter().find(|(k, _)| k == h).map(|(_, v)| v.as_str()) {
                    Some("off") => 0,
                    Some("read") => 1,
                    _ => 2,
                };
                v.push(Grant { path: h.to_string(), level, tool: Some(p.name) });
            }
        }
        for (key, level) in [("agents.read_paths", 1u8), ("agents.write_paths", 2)] {
            for p in split_paths(self.sget(key).unwrap_or("")) {
                if !v.iter().any(|g| g.path == p) {
                    v.push(Grant { path: p, level, tool: None });
                }
            }
        }
        v
    }

    /// Sets one access path's level: a preset path (0 off, 1 read, 2
    /// write) or an added one (1 read, 2 write, 0 removes it).
    fn set_grant(&mut self, k: usize, level: u8) {
        let Some(g) = self.grants().into_iter().nth(k) else { return };
        if g.tool.is_some() {
            let mut access = ffwd_agent::config::parse_preset_access(self.sget("agents.preset_access").unwrap_or(""));
            access.retain(|(p, _)| *p != g.path);
            if level != 2 {
                access.push((g.path.clone(), if level == 0 { "off".into() } else { "read".into() }));
            }
            let s = access.iter().map(|(p, l)| format!("{p}={l}")).collect::<Vec<_>>().join("; ");
            self.sset("agents.preset_access", &s);
            return;
        }
        for (key, keep) in [("agents.read_paths", level == 1), ("agents.write_paths", level == 2)] {
            let mut list = split_paths(self.sget(key).unwrap_or(""));
            list.retain(|p| *p != g.path);
            if keep {
                list.push(g.path.clone());
            }
            self.sset(key, &list.join("; "));
        }
    }

    /// The settings page: the window below the top bar, above the status
    /// bar.
    pub(super) fn settings_rect(&self) -> Rect {
        let top = self.lay.top.bottom();
        Rect::new(0, top, self.w as i32, (self.lay.status.y - top).max(1))
    }

    fn settings_nav_w(&self) -> i32 {
        self.px(236.0).min(self.w as i32 / 3)
    }

    /// The scrolling part: right of the sidebar, under the header.
    fn settings_content(&self) -> Rect {
        let r = self.settings_rect();
        let nav = self.settings_nav_w();
        let head = self.px(64.0);
        Rect::new(r.x + nav, r.y + head, r.w - nav, r.h - head)
    }

    /// The column the cards use inside the content.
    fn settings_column(&self) -> (i32, i32) {
        let c = self.settings_content();
        let pad = self.px(40.0);
        let w = (c.w - 2 * pad).clamp(self.px(320.0), self.px(860.0));
        (c.x + pad, w)
    }

    /// Keeps the selected row on screen.
    pub(super) fn settings_follow(&mut self) {
        self.settings_layout();
        let h = self.settings_content().h;
        if let Some(v) = &mut self.settings_view
            && let Some(&(y, rh)) = v.places.get(v.sel)
        {
            let margin = 24;
            if y - margin < v.scroll {
                v.scroll = (y - margin).max(0);
            } else if y + rh + margin > v.scroll + h {
                v.scroll = y + rh + margin - h;
            }
        }
    }

    fn settings_section(&mut self, section: usize) {
        self.commit_edit();
        if let Some(v) = &mut self.settings_view {
            v.section = section % SECTIONS.len();
            v.sel = 0;
            v.scroll = 0;
            v.edit = None;
            v.search = TextField::default();
            v.search_focus = false;
        }
        self.settings_dirty();
    }

    fn settings_dirty(&mut self) {
        let r = self.settings_rect();
        self.damage.rects.push(r);
    }

    /// Editing this repository's overrides (the scope picker).
    fn repo_scope(&self) -> bool {
        self.agent_scope_repo && !self.tabs.is_empty()
    }

    /// A value as the settings page shows it: agent keys in repository
    /// scope come from the repository's overrides, then the global value.
    fn sget(&self, key: &str) -> Option<&str> {
        if key.starts_with("git:") {
            return self.git_conf.get(key).map(String::as_str).filter(|v| !v.is_empty());
        }
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
        if let Some(k) = key.strip_prefix("git:") {
            return self.git_conf_set(k, value);
        }
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
            Row::Theme => match self.sget("ui.theme") {
                Some("dark") => 1,
                Some("light") => 2,
                _ => 0,
            },
            Row::Sidebar => self.side_style() as usize,
            Row::Layout => (self.layout == DetailsLayout::Beside) as usize,
            Row::Updates => (self.sget("update.check") == Some("false")) as usize,
            Row::SslVerify => (self.sget("git:global:http.sslVerify") == Some("false")) as usize,
            Row::SslBackend => match self.sget("git:global:http.sslBackend") {
                Some("openssl") => 1,
                Some("schannel") => 2,
                _ => 0,
            },
            Row::AutoFetch => match self.auto_fetch_setting() {
                0 => 0,
                1..=5 => 1,
                6..=10 => 2,
                _ => 3,
            },
            Row::Channel => match self.sget("update.channel") {
                Some("stable") => 1,
                Some("beta") => 2,
                _ => 0,
            },
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
            Row::Grant(k) => self.grants().get(k as usize).map_or(0, |g| g.level as usize),
            Row::WslDistro | Row::MergeTool | Row::DiffTool => {
                let key = self.list_row(row).map_or("", |l| l.0);
                let cur = self.sget(key).unwrap_or("").to_string();
                if cur.is_empty() { 0 } else { self.row_choices(row).iter().position(|d| *d == cur).unwrap_or(0) }
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

    pub(super) fn set_choice(&mut self, row: Row, i: usize) {
        match row {
            Row::Theme => {
                self.sset("ui.theme", ["system", "dark", "light"][i.min(2)]);
                self.apply_theme();
            }
            Row::Sidebar => self.set_side_style(super::SideStyle::KEYS[i.min(3)]),
            Row::Layout => self.set_layout(if i == 1 { DetailsLayout::Beside } else { DetailsLayout::Below }),
            Row::Updates => self.sset("update.check", if i == 1 { "false" } else { "true" }),
            // On is Git's default: the key goes.
            Row::SslVerify => self.sset("git:global:http.sslVerify", if i == 1 { "false" } else { "" }),
            Row::SslBackend => self.sset("git:global:http.sslBackend", ["", "openssl", "schannel"][i.min(2)]),
            Row::AutoFetch => {
                let m = [0u64, 5, 10, 30][i.min(3)];
                self.sset("remotes.auto_fetch", &m.to_string());
                self.auto_fetch_minutes.store(m, std::sync::atomic::Ordering::Relaxed);
            }
            Row::Channel => self.sset("update.channel", ["auto", "stable", "beta"][i.min(2)]),
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
            Row::Grant(k) => self.set_grant(k as usize, i.min(2) as u8),
            Row::WslDistro | Row::MergeTool | Row::DiffTool => {
                let key = self.list_row(row).map_or("", |l| l.0);
                let v = if i == 0 { String::new() } else { self.row_choices(row).get(i).cloned().unwrap_or_default() };
                self.sset(key, &v);
            }
            Row::AllowUnsandboxed => self.sset("agents.allow_unsandboxed", if i == 1 { "true" } else { "false" }),
            Row::Network => self.sset("agents.network", if i == 1 { "false" } else { "true" }),
            Row::GitAccess => self.sset("agents.git_access", if i == 1 { "write" } else { "read" }),
            Row::HomeAccess => self.sset("agents.home_access", ["private", "read", "write"][i.min(2)]),
            _ => {}
        }
        self.settings_dirty();
    }

    /// Switches the agent settings between all repositories and this one.
    pub(super) fn set_settings_scope(&mut self, repo: bool) {
        if repo && self.tabs.is_empty() {
            return self.set_notice("Open a repository first: its overrides belong to it.".into());
        }
        self.agent_scope_repo = repo;
        self.settings_dirty();
    }

    /// Opens the dropdown of visible row `i` under its control.
    fn settings_dropdown(&mut self, i: usize, at: Rect) {
        let Some(&row) = self.rows().get(i) else { return };
        let now = self.choice_now(row);
        let items = self
            .row_choices(row)
            .iter()
            .enumerate()
            .map(|(k, v)| {
                let label = self.choice_label(row, k, v);
                menu::MenuItem {
                    label: if k == now { format!("✓ {label}") } else { label },
                    act: Some(menu::MenuAct::SettingChoice(row, k)),
                    danger: false,
                }
            })
            .collect();
        self.open_menu(at.x, at.bottom() + self.px(4.0), items);
    }

    /// The scope picker's menu.
    fn settings_scope_menu(&mut self, at: Rect) {
        let repo = self.repo_scope();
        let name = self.r.repo_name.clone();
        let mark = |on: bool, t: String| if on { format!("✓ {t}") } else { t };
        let mut items = vec![menu::MenuItem {
            label: mark(!repo, "All repositories".into()),
            act: Some(menu::MenuAct::SettingScope(false)),
            danger: false,
        }];
        if !self.tabs.is_empty() {
            items.push(menu::MenuItem {
                label: mark(repo, format!("Only {name} (its own values)")),
                act: Some(menu::MenuAct::SettingScope(true)),
                danger: false,
            });
        }
        self.open_menu(at.x, at.bottom() + self.px(4.0), items);
    }

    /// Acts on visible row `i` (Enter, or a click on its control).
    fn settings_act(&mut self, i: usize) {
        let Some(&row) = self.rows().get(i) else { return };
        match row.ctl() {
            Ctl::Button => match row {
                // The answer shows under the button and in the status bar;
                // Settings stays open.
                Row::CheckUpdates if self.update_available => self.request_update_install(),
                Row::CheckUpdates => self.request_update_check(),
                Row::AboutLicense => self.open_beside_exe("LICENSE.txt"),
                Row::AboutNotices => self.open_beside_exe("THIRD-PARTY-NOTICES.txt"),
                Row::AboutSource => open_target(SOURCE_URL.to_string()),
                _ => open_settings_file(),
            },
            Ctl::Info => {}
            Ctl::Text => {
                let cur = if row == Row::AddGrant {
                    String::new()
                } else {
                    self.sget(row.key().unwrap_or_default()).unwrap_or("").to_string()
                };
                if let Some(v) = &mut self.settings_view {
                    v.edit = Some((i, TextField::new(&cur)));
                    v.search_focus = false;
                }
            }
            Ctl::Toggle => {
                let on = self.choice_now(row) == row.on_index();
                self.set_choice(row, if on { 1 - row.on_index() } else { row.on_index() });
            }
            Ctl::Dropdown => {
                let at =
                    self.settings_view.as_ref().and_then(|v| v.hits.iter().find(|h| h.1 == SHit::Drop(i)).map(|h| h.0));
                let at = at.unwrap_or_else(|| {
                    let c = self.settings_content();
                    Rect::new(c.x + c.w / 2, c.y + c.h / 3, 1, 1)
                });
                self.settings_dropdown(i, at);
            }
            Ctl::Segmented => {
                let n = self.row_choices(row).len().max(1);
                let next = (self.choice_now(row) + 1) % n;
                self.set_choice(row, next);
            }
        }
        self.settings_dirty();
    }

    /// Opens the system's picker for path row `i` on a worker (FF-57).
    fn settings_browse(&mut self, i: usize) {
        let Some(&row) = self.rows().get(i) else { return };
        let (Some(folder), Some(key)) = (browse_kind(row), row.key()) else { return };
        if cfg!(test) {
            return;
        }
        let title = match row {
            Row::Roots => "Add a folder with repositories",
            Row::AddGrant => "A folder the agent may use",
            Row::SslCaInfo => "Certificate authorities (a .pem file)",
            Row::GitPath => "The git program",
            _ => "Choose a program",
        };
        let sink = (self.sinks)(0);
        std::thread::spawn(move || sink(Msg::Picked(key, crate::platform::pick_path(folder, title))));
    }

    /// The picker's answer: a folder joins the repository folders or the
    /// agent's access list; a file becomes the row's value.
    pub(super) fn settings_picked(&mut self, key: &'static str, path: Option<String>) {
        let Some(p) = path else { return };
        match key {
            "repos.roots" => {
                let mut list = split_paths(self.sget(key).unwrap_or(""));
                if !list.contains(&p) {
                    list.push(p);
                }
                self.sset(key, &list.join("; "));
                self.scan_repos();
            }
            ADD_PATH_KEY => {
                if !self.grants().iter().any(|g| g.path == p) {
                    let mut list = split_paths(self.sget("agents.read_paths").unwrap_or(""));
                    list.push(p);
                    self.sset("agents.read_paths", &list.join("; "));
                }
            }
            k => self.sset(k, &p),
        }
        self.settings_dirty();
    }

    /// Reads Git's settings for the Git section on a worker: the global
    /// ones, this repository's identity and the version.
    fn load_git_conf(&mut self) {
        if cfg!(test) {
            return;
        }
        let sink = (self.sinks)(0);
        let repo = self.r.git.clone();
        let global = ffwd_core::git::Git { root: std::env::temp_dir() };
        std::thread::spawn(move || {
            let mut v = Vec::new();
            let get = |g: &ffwd_core::git::Git, scope: &str, k: &str| {
                g.output_limited(&["config", &format!("--{scope}"), "--get", k], 4096)
                    .map(|b| String::from_utf8_lossy(&b).trim().to_string())
                    .unwrap_or_default()
            };
            for k in
                ["user.name", "user.email", "credential.helper", "http.sslVerify", "http.sslCAInfo", "http.sslBackend"]
            {
                v.push((format!("git:global:{k}"), get(&global, "global", k)));
            }
            if let Some(r) = &repo {
                for k in ["user.name", "user.email"] {
                    v.push((format!("git:local:{k}"), get(r, "local", k)));
                }
            }
            let ver = global
                .output_limited(&["--version"], 4096)
                .map(|b| String::from_utf8_lossy(&b).trim().trim_start_matches("git version ").to_string())
                .unwrap_or_else(|_| "not found".into());
            v.push(("git:version".into(), ver));
            sink(Msg::GitConf(v));
        });
    }

    /// Saves one of Git's settings (`global:user.name`) on a worker; an
    /// empty value removes it. The page shows the new value at once.
    fn git_conf_set(&mut self, scoped: &str, value: &str) {
        self.git_conf.insert(format!("git:{scoped}"), value.to_string());
        if cfg!(test) {
            return;
        }
        let Some((scope, key)) = scoped.split_once(':') else { return };
        let g = match scope {
            "local" => match &self.r.git {
                Some(g) => g.clone(),
                None => return,
            },
            _ => ffwd_core::git::Git { root: std::env::temp_dir() },
        };
        let (scope, key, value) = (format!("--{scope}"), key.to_string(), value.to_string());
        let sink = (self.sinks)(0);
        std::thread::spawn(move || {
            let r = if value.is_empty() {
                // Nothing to remove is fine (exit 5).
                g.run_with(&["config", &scope, "--unset", &key], None, &[5], 4096).map(|_| ())
            } else {
                g.run_with(&["config", &scope, &key, &value], None, &[], 4096).map(|_| ())
            };
            sink(Msg::GitConfSaved(r.map_err(|e| format!("git config {key}: {e}"))));
        });
    }

    /// Removes this repository's value for visible row `i`.
    fn settings_reset(&mut self, i: usize) {
        if let Some(k) = self.rows().get(i).and_then(|r| r.agent_key())
            && let Some(r) = self.current_repo_settings_mut()
        {
            r.remove(k);
        }
        self.settings_dirty();
    }

    fn commit_edit(&mut self) {
        let rows = self.rows();
        let Some(v) = &mut self.settings_view else { return };
        if let Some((i, f)) = v.edit.take()
            && let Some(key) = rows.get(i).and_then(|r| r.key())
        {
            let text = f.text().trim().to_string();
            if key == ADD_PATH_KEY {
                // A new access path, read-only to start (FF-23).
                if !text.is_empty() && !self.grants().iter().any(|g| g.path == text) {
                    let mut list = split_paths(self.sget("agents.read_paths").unwrap_or(""));
                    list.push(text);
                    self.sset("agents.read_paths", &list.join("; "));
                }
                self.settings_dirty();
                return;
            }
            self.sset(key, &text);
            if key == "repos.roots" {
                self.scan_repos();
            }
        }
    }

    /// All keys go to the settings page while it is open.
    pub(super) fn settings_key(&mut self, k: Key) {
        let rows = self.rows();
        let Some(v) = &mut self.settings_view else { return };
        // A text being edited: the row's field or the search field.
        let editing_row = v.edit.is_some();
        if editing_row || v.search_focus {
            match k {
                Key::Enter | Key::Tab if editing_row => {
                    self.commit_edit();
                    return self.settings_dirty();
                }
                Key::Esc if editing_row => {
                    v.edit = None;
                    return self.settings_dirty();
                }
                Key::Esc => {
                    if v.search.text().is_empty() {
                        v.search_focus = false;
                    } else {
                        v.search = TextField::default();
                    }
                    v.sel = 0;
                    v.scroll = 0;
                    return self.settings_dirty();
                }
                Key::Enter | Key::Down => {
                    v.search_focus = false;
                    v.sel = 0;
                    return self.settings_dirty();
                }
                _ => {}
            }
            let f = match &mut v.edit {
                Some((_, f)) => f,
                None => &mut v.search,
            };
            match k {
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
            if !editing_row {
                // The results change with every letter.
                let n = self.visible_len();
                if let Some(v) = &mut self.settings_view {
                    v.sel = v.sel.min(n.saturating_sub(1));
                    v.scroll = 0;
                }
            }
            return self.settings_dirty();
        }
        let n = rows.len();
        let sel = v.sel.min(n.saturating_sub(1));
        v.keys = true;
        match k {
            Key::Esc | Key::Ctrl(',') => return self.close_settings(),
            Key::Ctrl('f') => v.search_focus = true,
            Key::NextTab => {
                let s = v.section;
                self.settings_section(s + 1);
            }
            Key::PrevTab => {
                let s = v.section;
                self.settings_section(s + SECTIONS.len() - 1);
            }
            Key::Up | Key::BackTab => v.sel = sel.saturating_sub(1),
            Key::Down | Key::Tab => v.sel = (sel + 1).min(n.saturating_sub(1)),
            Key::Delete if self.agent_scope_repo => self.settings_reset(sel),
            Key::PageUp | Key::Home => v.sel = 0,
            Key::PageDown | Key::End => v.sel = n.saturating_sub(1),
            Key::Enter | Key::Char(' ') if n > 0 => self.settings_act(sel),
            Key::Left | Key::Right if n > 0 && !matches!(rows[sel].ctl(), Ctl::Text | Ctl::Button | Ctl::Info) => {
                let c = self.row_choices(rows[sel]).len().max(1);
                let cur = self.choice_now(rows[sel]);
                let next = if matches!(k, Key::Left) { (cur + c - 1) % c } else { (cur + 1) % c };
                self.set_choice(rows[sel], next);
            }
            Key::Char(c) if n > 0 && rows[sel].key().is_some() => {
                // Typing on a text row starts editing it.
                self.settings_act(sel);
                if let Some(SettingsView { edit: Some((_, f)), .. }) = &mut self.settings_view {
                    f.select_all();
                    f.insert(c.encode_utf8(&mut [0; 4]));
                }
            }
            Key::Char(c) => {
                // Typing elsewhere searches.
                v.search_focus = true;
                v.search.insert(c.encode_utf8(&mut [0; 4]));
                v.sel = 0;
                v.scroll = 0;
            }
            _ => {}
        }
        self.settings_follow();
        self.settings_dirty();
    }

    fn visible_len(&self) -> usize {
        self.visible().len()
    }

    pub(super) fn settings_wheel(&mut self, lines: i64) {
        self.settings_layout();
        let h = self.settings_content().h;
        let step = self.px(48.0);
        if let Some(v) = &mut self.settings_view {
            let max = (v.content_h - h).max(0);
            v.scroll = (v.scroll + lines as i32 * step).clamp(0, max);
        }
        self.settings_dirty();
    }

    pub(super) fn settings_click(&mut self, x: i32, y: i32) {
        let r = self.settings_rect();
        if !r.contains(x, y) {
            self.commit_edit();
            return self.close_settings();
        }
        let hit = self.settings_view.as_ref().and_then(|v| v.hits.iter().find(|h| h.0.contains(x, y)).copied());
        let Some((rect, hit)) = hit else {
            self.commit_edit();
            if let Some(v) = &mut self.settings_view {
                v.search_focus = false;
            }
            return self.settings_dirty();
        };
        if !matches!(hit, SHit::Field(_)) {
            self.commit_edit();
        }
        let row_of = |h: SHit| match h {
            SHit::Seg(i, _) | SHit::Toggle(i) | SHit::Drop(i) | SHit::Field(i) | SHit::Button(i) | SHit::Reset(i) => {
                Some(i)
            }
            _ => None,
        };
        if let (Some(i), Some(v)) = (row_of(hit), &mut self.settings_view) {
            v.sel = i;
            v.search_focus = false;
        }
        match hit {
            SHit::Nav(s) => self.settings_section(s),
            SHit::Search => {
                if let Some(v) = &mut self.settings_view {
                    v.search_focus = true;
                    v.edit = None;
                }
            }
            SHit::Scope => self.settings_scope_menu(rect),
            SHit::Close => return self.close_settings(),
            SHit::Seg(i, c) => {
                if let Some(&row) = self.rows().get(i) {
                    self.set_choice(row, c);
                }
            }
            SHit::Drop(i) => self.settings_dropdown(i, rect),
            SHit::Toggle(i) | SHit::Button(i) => self.settings_act(i),
            SHit::Field(i) => {
                let editing = self.settings_view.as_ref().is_some_and(|v| v.edit.as_ref().is_some_and(|e| e.0 == i));
                if !editing {
                    self.settings_act(i);
                }
            }
            SHit::Reset(i) => self.settings_reset(i),
            SHit::EditThere => self.set_settings_scope(true),
            SHit::TurnOn => self.set_choice(Row::AgentsOn, 1),
            SHit::Browse(i) => self.settings_browse(i),
        }
        self.settings_dirty();
    }

    /// Splits `text` into at most `max` lines of `w` pixels (the last one
    /// ends in … when the text is longer).
    fn settings_wrap(&mut self, text: &str, size: f32, w: f32, max: usize) -> Vec<String> {
        let ui = self.f.ui;
        let mut lines: Vec<String> = Vec::new();
        let mut cur = String::new();
        for word in text.split_whitespace() {
            let next = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
            if self.te.measure(ui, size, &next) <= w || cur.is_empty() {
                cur = next;
            } else {
                lines.push(std::mem::take(&mut cur));
                cur = word.to_string();
                if lines.len() == max {
                    break;
                }
            }
        }
        if lines.len() < max && !cur.is_empty() {
            lines.push(cur);
        } else if lines.len() == max
            && let Some(last) = lines.last_mut()
        {
            last.push('…');
        }
        lines
    }

    /// Lays the content out: each visible row's y (from the content top)
    /// and height; section headings and the agents' status line take room
    /// too.
    fn settings_layout(&mut self) {
        let visible = self.visible();
        let (_, col_w) = self.settings_column();
        let ctl_w = self.settings_ctl_w(col_w);
        let text_w = (col_w - ctl_w - self.px(56.0)) as f32;
        let mut places = Vec::with_capacity(visible.len());
        let mut y = self.px(28.0);
        let mut last_section = None;
        for (k, &(si, row)) in visible.iter().enumerate() {
            if last_section != Some(si) {
                if last_section.is_some() {
                    y += self.px(28.0);
                }
                y += self.settings_heading_h(si, col_w);
                last_section = Some(si);
            }
            let hint = self.row_hint(row);
            let lines = self.settings_wrap(&hint, self.pf(12.0), text_w, 3).len().max(1) as i32;
            let mut h = self.px(26.0) + lines * self.px(17.0) + self.px(22.0);
            if row.agent_key().is_some() && (self.overridden(row) || self.repo_scope()) {
                h += self.px(20.0);
            }
            let _ = k;
            places.push((y, h));
            y += h;
        }
        y += self.px(40.0);
        let content_h = self.settings_content().h;
        if let Some(v) = &mut self.settings_view {
            v.places = places;
            v.content_h = y;
            let max = (y - content_h).max(0);
            v.scroll = v.scroll.min(max);
        }
    }

    fn settings_ctl_w(&self, col_w: i32) -> i32 {
        ((col_w as f32 * 0.4) as i32).clamp(self.px(240.0), self.px(360.0))
    }

    /// A section heading's height: title, about text, and on agent
    /// sections the status of what is in effect.
    fn settings_heading_h(&mut self, si: usize, col_w: i32) -> i32 {
        let s = &SECTIONS[si];
        let lines = self.settings_wrap(s.about, self.pf(12.5), col_w as f32, 3).len() as i32;
        let mut h = self.px(30.0) + lines * self.px(18.0) + self.px(16.0);
        if s.agents {
            h += self.px(70.0);
        }
        h
    }

    pub(super) fn paint_settings(&mut self, cv: &mut Canvas) {
        self.settings_layout();
        let visible = self.visible();
        let Some(view) = self.settings_view.take() else { return };
        let r = self.settings_rect();
        let old = cv.set_clip(cv.clip().intersect(r));
        let (ui, semi, mono) = (self.f.ui, self.f.semi, self.f.mono);
        let mut hits: Vec<(Rect, SHit)> = Vec::new();
        cv.fill(r, BG());

        // Sidebar: groups and sections.
        let nav = Rect::new(r.x, r.y, self.settings_nav_w(), r.h);
        cv.fill(nav, BG_SIDEBAR());
        cv.fill(Rect::new(nav.right() - 1, nav.y, 1, nav.h), BORDER_SOFT());
        let mut y = nav.y + self.px(20.0);
        let nx = nav.x + self.px(14.0);
        let searching = !view.search.text().trim().is_empty();
        let shown: Vec<usize> = visible.iter().map(|v| v.0).collect();
        self.te.draw(
            cv,
            semi,
            self.pf(17.0),
            nx as f32 + self.pf(6.0),
            y as f32 + self.pf(16.0),
            "Settings",
            TEXT(),
            200.0,
        );
        y += self.px(40.0);
        for (gi, &(title, from, to)) in GROUPS.iter().enumerate() {
            if gi > 0 {
                y += self.px(14.0);
            }
            let gy = y as f32 + self.pf(14.0);
            let e = self.te.draw(
                cv,
                semi,
                self.pf(10.5),
                nx as f32 + self.pf(6.0),
                gy,
                &title.to_uppercase(),
                TEXT_4(),
                200.0,
            );
            if from == FIRST_AGENT_SECTION {
                let label = "EXPERIMENTAL";
                let pw = self.te.measure(semi, self.pf(9.0), label) + self.pf(10.0);
                let pr = Rect::new((e + self.pf(8.0)) as i32, y + self.px(3.0), pw as i32, self.px(15.0));
                cv.fill_round(pr, self.pf(7.0), TAG_BG());
                self.te.draw(
                    cv,
                    semi,
                    self.pf(9.0),
                    pr.x as f32 + self.pf(5.0),
                    pr.y as f32 + self.pf(11.0),
                    label,
                    TAG_FG(),
                    pw,
                );
            }
            y += self.px(24.0);
            for (si, s) in SECTIONS.iter().enumerate().take(to).skip(from) {
                let item = Rect::new(nav.x + self.px(8.0), y, nav.w - self.px(16.0), self.px(32.0));
                let on = !searching && si == view.section;
                let hot = self.is_hot(Hot::Setting(hits.len(), 0));
                if on {
                    cv.fill_round(item, self.pf(7.0), BG_SELECT_DIM());
                    cv.fill_round(
                        Rect::new(item.x, item.y + self.px(8.0), self.px(3.0), item.h - self.px(16.0)),
                        self.pf(1.5),
                        ACCENT(),
                    );
                } else if hot {
                    cv.fill_round(item, self.pf(7.0), BG_HOVER());
                }
                let dim = searching && !shown.contains(&si);
                let color = if on {
                    TEXT()
                } else if dim {
                    TEXT_4()
                } else {
                    TEXT_2()
                };
                let cy = item.y as f32 + item.h as f32 / 2.0;
                self.glyph(
                    cv,
                    s.glyph,
                    item.x as f32 + self.pf(18.0),
                    cy,
                    self.pf(6.0),
                    if on { ACCENT() } else { TEXT_3() },
                );
                let font = if on { semi } else { ui };
                self.te.draw(
                    cv,
                    font,
                    self.pf(13.0),
                    item.x as f32 + self.pf(34.0),
                    cy + self.pf(4.5),
                    s.nav,
                    color,
                    item.w as f32,
                );
                hits.push((item, SHit::Nav(si)));
                y += self.px(34.0);
            }
        }
        let ver = format!("ffwd {}", env!("CARGO_PKG_VERSION"));
        self.te.draw(
            cv,
            mono,
            self.pf(11.0),
            nx as f32 + self.pf(6.0),
            (nav.bottom() - self.px(16.0)) as f32,
            &ver,
            TEXT_4(),
            200.0,
        );

        // Header: search, the scope picker on agent pages, Done.
        let head = Rect::new(nav.right(), r.y, r.w - nav.w, self.px(64.0));
        cv.fill(Rect::new(head.x, head.bottom() - 1, head.w, 1), BORDER_SOFT());
        let (col_x, col_w) = self.settings_column();
        let sr = Rect::new(col_x, head.y + self.px(14.0), (col_w / 2).min(self.px(380.0)), self.px(36.0));
        let focus = view.search_focus;
        cv.round_border(sr, self.pf(8.0), if focus { ACCENT() } else { BORDER() }, BG_INPUT());
        let scy = sr.y as f32 + sr.h as f32 / 2.0;
        self.glyph(cv, Glyph::Search, sr.x as f32 + self.pf(18.0), scy, self.pf(6.0), TEXT_3());
        let tx = sr.x as f32 + self.pf(34.0);
        let base = (scy + self.pf(4.5)).round();
        if view.search.text().is_empty() {
            self.te.draw(cv, ui, self.pf(13.0), tx, base, "Search settings", TEXT_4(), sr.w as f32 - self.pf(44.0));
        } else {
            self.te.draw(cv, ui, self.pf(13.0), tx, base, view.search.text(), TEXT(), sr.w as f32 - self.pf(44.0));
        }
        if focus {
            let cx = tx + self.te.measure(ui, self.pf(13.0), &view.search.text()[..view.search.cursor()]);
            cv.fill(Rect::new(cx as i32 + 1, sr.y + self.px(9.0), self.px(1.5).max(1), sr.h - self.px(18.0)), ACCENT());
        }
        hits.push((sr, SHit::Search));
        // Done (Esc).
        let dl = "Done";
        let dw = self.te.measure(semi, self.pf(12.5), dl) + self.pf(28.0);
        let dr = Rect::new(head.right() - self.px(24.0) - dw as i32, sr.y, dw as i32, sr.h);
        let (dbg, dborder) = self.control_colors(Hot::Setting(hits.len(), 0), ACCENT_SOFT_HOVER(), ACCENT_BORDER());
        cv.round_border(dr, self.pf(8.0), dborder, dbg);
        self.te.draw(cv, semi, self.pf(12.5), dr.x as f32 + self.pf(14.0), base, dl, ACCENT_FG(), dw);
        hits.push((dr, SHit::Close));
        // The scope picker: agent settings apply everywhere or here.
        let agents_on_screen =
            visible.iter().any(|(si, _)| SECTIONS[*si].agents) || (!searching && SECTIONS[view.section].agents);
        if agents_on_screen {
            let repo = self.repo_scope();
            let name = self.r.repo_name.clone();
            let label = if repo { format!("Applies to: {name} only") } else { "Applies to: all repositories".into() };
            let lw = self.te.measure(semi, self.pf(12.5), &label) + self.pf(44.0);
            let pr = Rect::new(dr.x - self.px(12.0) - lw as i32, sr.y, lw as i32, sr.h);
            let (pbg, pborder) = self.control_colors(
                Hot::Setting(hits.len(), 0),
                if repo { ACCENT_SOFT() } else { BG_INPUT() },
                if repo { ACCENT_BORDER() } else { BORDER() },
            );
            cv.round_border(pr, self.pf(8.0), pborder, pbg);
            self.te.draw(
                cv,
                semi,
                self.pf(12.5),
                pr.x as f32 + self.pf(14.0),
                base,
                &label,
                if repo { ACCENT_FG() } else { TEXT() },
                lw,
            );
            self.chevron_down(cv, pr.right() as f32 - self.pf(16.0), scy, TEXT_3());
            hits.push((pr, SHit::Scope));
        }

        // Content.
        let content = self.settings_content();
        let clip_c = cv.clip().intersect(content);
        let old_c = cv.set_clip(clip_c);
        let ctl_w = self.settings_ctl_w(col_w);
        let text_w = (col_w - ctl_w - self.px(56.0)) as f32;
        let top = content.y - view.scroll;
        let mut last_section = None;
        // The cards first, one per section, so the rows paint on them.
        let mut cards: Vec<(usize, Rect)> = Vec::new();
        for (i, &(si, _)) in visible.iter().enumerate() {
            let Some(&(ry, rh)) = view.places.get(i) else { break };
            let rr = Rect::new(col_x, top + ry, col_w, rh);
            match cards.last_mut() {
                Some((s, c)) if *s == si => *c = Rect::new(c.x, c.y, c.w, rr.bottom() - c.y),
                _ => cards.push((si, rr)),
            }
        }
        for (_, c) in &cards {
            if !c.intersect(content).is_empty() {
                cv.round_border(*c, self.pf(10.0), BORDER(), BG_PANEL());
            }
        }
        let repo_name = self.r.repo_name.clone();
        let rows_n = visible.len();
        for (i, &(si, row)) in visible.iter().enumerate() {
            let Some(&(ry, rh)) = view.places.get(i) else { break };
            let y = top + ry;
            if last_section != Some(si) {
                // The section heading sits above its first row.
                let hh = self.settings_heading_h(si, col_w);
                let hy = y - hh;
                let s = &SECTIONS[si];
                if hy + hh > content.y && hy < content.bottom() {
                    self.te.draw(
                        cv,
                        semi,
                        self.pf(16.0),
                        col_x as f32,
                        hy as f32 + self.pf(20.0),
                        s.nav,
                        TEXT(),
                        col_w as f32,
                    );
                    let mut ay = hy as f32 + self.pf(42.0);
                    for line in self.settings_wrap(s.about, self.pf(12.5), col_w as f32, 3) {
                        self.te.draw(cv, ui, self.pf(12.5), col_x as f32, ay, &line, TEXT_3(), col_w as f32);
                        ay += self.pf(18.0);
                    }
                    if s.agents {
                        self.paint_agent_status(
                            cv,
                            Rect::new(col_x, (ay + self.pf(2.0)) as i32, col_w, self.px(50.0)),
                            &mut hits,
                        );
                    }
                }
                last_section = Some(si);
            }
            let rr = Rect::new(col_x, y, col_w, rh);
            if rr.bottom() < content.y || rr.y > content.bottom() {
                continue;
            }
            if i == view.sel && view.keys && !view.search_focus {
                cv.fill_round(rr.inset(self.px(4.0), self.px(4.0)), self.pf(7.0), BG_SELECT_DIM());
            }
            // A line between rows of a card.
            if i > 0 && visible[i - 1].0 == si {
                cv.fill(Rect::new(rr.x + self.px(16.0), rr.y, rr.w - self.px(32.0), 1), BORDER_SOFT());
            }
            let lx = rr.x as f32 + self.pf(20.0);
            let label_y = rr.y as f32 + self.pf(26.0);
            let label = self.row_label(row);
            let lfont = if matches!(row, Row::Grant(_)) { mono } else { semi };
            self.te.draw(cv, lfont, self.pf(13.5), lx, label_y, &label, TEXT(), text_w);
            let mut hy = label_y + self.pf(19.0);
            let hint = self.row_hint(row);
            for line in self.settings_wrap(&hint, self.pf(12.0), text_w, 3) {
                self.te.draw(cv, ui, self.pf(12.0), lx, hy, &line, TEXT_3(), text_w);
                hy += self.pf(17.0);
            }
            // Where the value comes from (agent settings).
            if row.agent_key().is_some() && (self.overridden(row) || self.repo_scope()) {
                let over = self.overridden(row);
                let (text, color, link) = match (self.repo_scope(), over) {
                    (true, true) => (format!("Set for {repo_name}"), ACCENT_FG(), Some(("Reset", SHit::Reset(i)))),
                    (true, false) => ("From all repositories".to_string(), TEXT_4(), None),
                    (false, _) => (format!("{repo_name} has its own value"), MOD(), Some(("Show it", SHit::EditThere))),
                };
                let by = hy + self.pf(3.0);
                cv.fill_circle(lx + self.pf(3.0), by - self.pf(4.0), self.pf(3.0), color);
                let e = self.te.draw(cv, ui, self.pf(11.5), lx + self.pf(12.0), by, &text, color, text_w);
                if let Some((l, h)) = link {
                    let lw = self.te.measure(semi, self.pf(11.5), l);
                    let lr = Rect::new(
                        (e + self.pf(10.0)) as i32,
                        (by - self.pf(13.0)) as i32,
                        lw as i32 + 4,
                        self.px(18.0),
                    );
                    let lc = if self.is_hot(Hot::Setting(hits.len(), 0)) { TEXT() } else { ACCENT() };
                    self.te.draw(cv, semi, self.pf(11.5), lr.x as f32 + 2.0, by, l, lc, lw + 4.0);
                    hits.push((lr, h));
                }
            }
            // The control, right-aligned.
            let ch = self.px(32.0);
            let ctl = Rect::new(
                rr.right() - self.px(20.0) - ctl_w,
                rr.y + (self.px(22.0) - ch / 2).max(self.px(10.0)),
                ctl_w,
                ch,
            );
            let cbase = (ctl.y as f32 + ctl.h as f32 / 2.0 + self.pf(4.5)).round();
            match row.ctl() {
                Ctl::Toggle => {
                    let on = self.choice_now(row) == row.on_index();
                    let tw = self.px(42.0);
                    let th = self.px(24.0);
                    let t = Rect::new(ctl.right() - tw, ctl.y + (ctl.h - th) / 2, tw, th);
                    let hot = self.is_hot(Hot::Setting(hits.len(), 0));
                    let (bg, knob) = match (on, hot) {
                        (true, _) => (ACCENT(), ACCENT_TEXT()),
                        (false, true) => (BORDER_HOVER(), TEXT()),
                        (false, false) => (BORDER_STRONG(), TEXT_2()),
                    };
                    cv.fill_round(t, th as f32 / 2.0, bg);
                    let kr = th as f32 / 2.0 - self.pf(3.5);
                    let kx = if on { t.right() as f32 - th as f32 / 2.0 } else { t.x as f32 + th as f32 / 2.0 };
                    cv.fill_circle(kx, t.y as f32 + th as f32 / 2.0, kr, knob);
                    let state = if on { "On" } else { "Off" };
                    let sw = self.te.measure(ui, self.pf(12.5), state);
                    self.te.draw(
                        cv,
                        ui,
                        self.pf(12.5),
                        t.x as f32 - self.pf(10.0) - sw,
                        cbase,
                        state,
                        if on { TEXT() } else { TEXT_3() },
                        sw + 2.0,
                    );
                    // The whole row's right part toggles.
                    hits.push((Rect::new(t.x - self.px(48.0), ctl.y, t.w + self.px(48.0), ctl.h), SHit::Toggle(i)));
                }
                Ctl::Dropdown => {
                    let now = self.choice_now(row);
                    let choices = self.row_choices(row);
                    let label = choices.get(now).map(|v| self.choice_label(row, now, v)).unwrap_or_default();
                    let (bg, border) = self.control_colors(Hot::Setting(hits.len(), 0), BG_INPUT(), BORDER());
                    cv.round_border(ctl, self.pf(7.0), border, bg);
                    self.te.draw(
                        cv,
                        ui,
                        self.pf(12.5),
                        ctl.x as f32 + self.pf(12.0),
                        cbase,
                        &label,
                        TEXT(),
                        ctl.w as f32 - self.pf(40.0),
                    );
                    self.chevron_down(
                        cv,
                        ctl.right() as f32 - self.pf(16.0),
                        ctl.y as f32 + ctl.h as f32 / 2.0,
                        TEXT_3(),
                    );
                    hits.push((ctl, SHit::Drop(i)));
                }
                Ctl::Segmented => {
                    let choices = self.row_choices(row);
                    let now = self.choice_now(row);
                    let seg_w = ctl.w / choices.len().max(1) as i32;
                    cv.round_border(ctl, self.pf(7.0), BORDER(), BG_INPUT());
                    for (c, name) in choices.iter().enumerate() {
                        let s = Rect::new(ctl.x + c as i32 * seg_w, ctl.y, seg_w, ctl.h);
                        if c == now {
                            cv.fill_round(s.inset(self.px(3.0), self.px(3.0)), self.pf(5.0), BG_CHOSEN());
                        } else if self.is_hot(Hot::Setting(hits.len(), 0)) {
                            cv.fill_round(s.inset(self.px(3.0), self.px(3.0)), self.pf(5.0), CONTROL_HOVER());
                        }
                        let f = if c == now { semi } else { ui };
                        let tw = self.te.measure(f, self.pf(12.5), name).min(s.w as f32 - 8.0);
                        self.te.draw(
                            cv,
                            f,
                            self.pf(12.5),
                            s.x as f32 + (s.w as f32 - tw) / 2.0,
                            cbase,
                            name,
                            if c == now { TEXT() } else { TEXT_3() },
                            s.w as f32 - 8.0,
                        );
                        hits.push((s, SHit::Seg(i, c)));
                    }
                }
                Ctl::Info => {
                    let v = match row {
                        Row::AboutVersion => format!("ffwd {}", env!("CARGO_PKG_VERSION")),
                        _ => self.sget("git:version").unwrap_or("…").to_string(),
                    };
                    let tw = self.te.measure(mono, self.pf(12.5), &v).min(ctl.w as f32);
                    self.te.draw(cv, mono, self.pf(12.5), ctl.right() as f32 - tw, cbase, &v, TEXT_2(), ctl.w as f32);
                }
                Ctl::Text => {
                    // Path rows get Browse… beside their field.
                    let ctl = match browse_kind(row) {
                        Some(_) => {
                            let bl = "Browse…";
                            let bw = (self.te.measure(semi, self.pf(12.5), bl) + self.pf(24.0)) as i32;
                            let b = Rect::new(ctl.right() - bw, ctl.y, bw, ctl.h);
                            let (bg, border) = self.control_colors(Hot::Setting(hits.len(), 0), BG_BAR(), BORDER());
                            cv.round_border(b, self.pf(7.0), border, bg);
                            self.te.draw(
                                cv,
                                semi,
                                self.pf(12.5),
                                b.x as f32 + self.pf(12.0),
                                cbase,
                                bl,
                                TEXT(),
                                bw as f32,
                            );
                            hits.push((b, SHit::Browse(i)));
                            Rect::new(ctl.x, ctl.y, ctl.w - bw - self.px(8.0), ctl.h)
                        }
                        None => ctl,
                    };
                    let editing = view.edit.as_ref().filter(|(j, _)| *j == i).map(|(_, f)| f.clone());
                    let hot = self.is_hot(Hot::Setting(hits.len(), 0));
                    let border = if editing.is_some() {
                        ACCENT()
                    } else if hot {
                        BORDER_HOVER()
                    } else {
                        BORDER()
                    };
                    cv.round_border(ctl, self.pf(7.0), border, BG_INPUT());
                    let tx = ctl.x as f32 + self.pf(12.0);
                    let maxw = ctl.w as f32 - self.pf(24.0);
                    match editing {
                        Some(f) => {
                            if f.all_selected() {
                                let w = self.te.measure(ui, self.pf(12.5), f.text()).min(maxw);
                                cv.fill_round(
                                    Rect::new(tx as i32 - 2, ctl.y + self.px(6.0), w as i32 + 4, ctl.h - self.px(12.0)),
                                    self.pf(3.0),
                                    BG_TEXT_SELECT(),
                                );
                            }
                            self.te.draw(cv, ui, self.pf(12.5), tx, cbase, f.text(), TEXT(), maxw);
                            let cx = tx + self.te.measure(ui, self.pf(12.5), &f.text()[..f.cursor()]);
                            cv.fill(
                                Rect::new(
                                    cx as i32 + 1,
                                    ctl.y + self.px(7.0),
                                    self.px(1.5).max(1),
                                    ctl.h - self.px(14.0),
                                ),
                                ACCENT(),
                            );
                        }
                        None => {
                            let v = if row == Row::AddGrant {
                                String::new()
                            } else {
                                self.sget(row.key().unwrap_or_default()).unwrap_or("").to_string()
                            };
                            // The agent's own command runs while this is empty.
                            let preset = (row == Row::AgentCommand)
                                .then(|| self.sget("agents.preset").and_then(ffwd_agent::presets::find))
                                .flatten();
                            let ph = match preset {
                                Some(p) => format!("{}: {}", p.name, p.command),
                                None => self.row_placeholder(row),
                            };
                            if v.is_empty() {
                                self.te.draw(cv, ui, self.pf(12.5), tx, cbase, &ph, TEXT_4(), maxw);
                            } else {
                                self.te.draw(cv, ui, self.pf(12.5), tx, cbase, &v, TEXT(), maxw);
                            }
                        }
                    }
                    hits.push((ctl, SHit::Field(i)));
                }
                Ctl::Button => {
                    let label = match row {
                        Row::CheckUpdates if self.update_available => "Install update",
                        Row::CheckUpdates => "Check for updates",
                        Row::AboutLicense => "Open licence",
                        Row::AboutNotices => "Open licences",
                        Row::AboutSource => "Open repository",
                        _ => "Open settings.toml",
                    };
                    let w = self.te.measure(semi, self.pf(12.5), label) + self.pf(28.0);
                    let b = Rect::new(ctl.right() - w as i32, ctl.y, w as i32, ctl.h);
                    let (bg, border) = self.control_colors(Hot::Setting(hits.len(), 0), BG_BAR(), BORDER());
                    cv.round_border(b, self.pf(8.0), border, bg);
                    self.te.draw(cv, semi, self.pf(12.5), b.x as f32 + self.pf(14.0), cbase, label, TEXT(), w);
                    hits.push((b, SHit::Button(i)));
                }
            }
        }
        if rows_n == 0 {
            let msg = if searching { "No setting matches." } else { "" };
            self.te.draw(
                cv,
                ui,
                self.pf(13.0),
                col_x as f32,
                content.y as f32 + self.pf(40.0),
                msg,
                TEXT_3(),
                col_w as f32,
            );
        }
        // A thin scroll indicator.
        if view.content_h > content.h {
            let track = Rect::new(
                content.right() - self.px(6.0),
                content.y + self.px(4.0),
                self.px(3.0),
                content.h - self.px(8.0),
            );
            let th = (track.h as f32 * content.h as f32 / view.content_h as f32) as i32;
            let ty = track.y
                + ((track.h - th) as f32 * view.scroll as f32 / (view.content_h - content.h).max(1) as f32) as i32;
            cv.fill_round(Rect::new(track.x, ty, track.w, th.max(self.px(20.0))), self.pf(1.5), BORDER_STRONG());
        }
        cv.set_clip(old_c);
        cv.set_clip(old);
        // Only the targets inside the content (or the fixed parts) count.
        let hits = hits
            .into_iter()
            .filter(|(h, k)| {
                !matches!(
                    k,
                    SHit::Seg(..)
                        | SHit::Toggle(_)
                        | SHit::Drop(_)
                        | SHit::Field(_)
                        | SHit::Button(_)
                        | SHit::Reset(_)
                        | SHit::EditThere
                        | SHit::TurnOn
                        | SHit::Browse(_)
                ) || !h.intersect(content).is_empty()
            })
            .collect();
        self.settings_view = Some(SettingsView { hits, ..view });
    }

    /// The agent settings in effect for the open repository, in one or two
    /// lines: on or off, the tool and its sandbox, and why it cannot run.
    fn paint_agent_status(&mut self, cv: &mut Canvas, r: Rect, hits: &mut Vec<(Rect, SHit)>) {
        let (ui, semi) = (self.f.ui, self.f.semi);
        let (cfg, errors) = self.agent_config();
        let name = self.r.repo_name.clone();
        let refusal = cfg.refusal();
        let (dot, head) = match (&refusal, cfg.enabled) {
            (_, false) => (TEXT_4(), format!("Agents are off for {name}")),
            (Some(_), true) => (MOD(), format!("Agents are on for {name}, but cannot run yet")),
            (None, true) => (ADD(), format!("Agents are on for {name}")),
        };
        cv.round_border(r, self.pf(8.0), BORDER_SOFT(), BG_SIDEBAR());
        let cy = r.y as f32 + self.pf(18.0);
        cv.fill_circle(r.x as f32 + self.pf(16.0), cy - self.pf(4.0), self.pf(4.0), dot);
        let mut text_w = r.w as f32 - self.pf(40.0);
        if !cfg.enabled {
            // Off: one click turns them on (for every repository, or for
            // this one, as the scope picker says).
            let l = "Turn on";
            let w = self.te.measure(semi, self.pf(12.5), l) + self.pf(24.0);
            let b = Rect::new(
                r.right() - self.px(10.0) - w as i32,
                r.y + (r.h - self.px(30.0)) / 2,
                w as i32,
                self.px(30.0),
            );
            let (bg, border) = self.control_colors(Hot::Setting(hits.len(), 0), ACCENT_SOFT_HOVER(), ACCENT_BORDER());
            cv.round_border(b, self.pf(7.0), border, bg);
            let bb = (b.y as f32 + b.h as f32 / 2.0 + self.pf(4.5)).round();
            self.te.draw(cv, semi, self.pf(12.5), b.x as f32 + self.pf(12.0), bb, l, ACCENT_FG(), w);
            hits.push((b, SHit::TurnOn));
            text_w -= w + self.pf(16.0);
        }
        self.te.draw(cv, semi, self.pf(12.5), r.x as f32 + self.pf(28.0), cy, &head, TEXT(), text_w);
        let detail = match (refusal, errors.first()) {
            _ if !cfg.enabled => {
                "An agent works on a task in its own worktree and sandbox; you review and merge.".into()
            }
            (Some(why), _) => why,
            (None, Some(e)) => format!("Settings: {e}"),
            (None, None) => cfg.summary(),
        };
        self.te.draw(cv, ui, self.pf(12.0), r.x as f32 + self.pf(28.0), cy + self.pf(19.0), &detail, TEXT_3(), text_w);
    }

    /// A small downward chevron (dropdowns).
    fn chevron_down(&self, cv: &mut Canvas, cx: f32, cy: f32, color: u32) {
        let s = self.pf(3.5);
        cv.stroke(&[(cx - s, cy - s / 2.0), (cx, cy + s / 2.0), (cx + s, cy - s / 2.0)], self.pf(1.4), color);
    }

    /// Tests: the click target of visible row `row` (its first match):
    /// the segment `choice` of a segmented row, else the row's control.
    #[cfg(test)]
    pub(super) fn settings_target(&self, row: Row, choice: usize) -> Option<Rect> {
        let i = self.rows().iter().position(|r| *r == row)?;
        let v = self.settings_view.as_ref()?;
        v.hits
            .iter()
            .find(|h| match h.1 {
                SHit::Seg(j, c) => j == i && c == choice,
                SHit::Toggle(j) | SHit::Drop(j) | SHit::Field(j) | SHit::Button(j) => j == i,
                _ => false,
            })
            .map(|h| h.0)
    }

    /// Tests: shows the section named `nav`.
    #[cfg(test)]
    pub(super) fn settings_go(&mut self, nav: &str) {
        let i = SECTIONS.iter().position(|s| s.nav == nav).expect("a section");
        self.settings_section(i);
    }

    /// Tests: the section shown.
    #[cfg(test)]
    pub(super) fn settings_section_now(&self) -> Option<&'static str> {
        self.settings_view.as_ref().map(|v| SECTIONS[v.section].nav)
    }
}

/// Opens settings.toml in the default editor (creating it with a short,
/// commented template first). Runs on a worker: file and process work stay
/// off the UI thread.
/// ffwd's own licence, for builds without LICENSE.txt beside them.
const LICENSE: &str = include_str!("../../../LICENSE");

/// Where ffwd's source lives.
const SOURCE_URL: &str = "https://github.com/rennerdo30/ffwd";

/// Opens a file or a web address with the system's handler, off the UI
/// thread.
fn open_target(target: String) {
    if cfg!(test) {
        return;
    }
    std::thread::spawn(move || {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // CREATE_NO_WINDOW: no console flashes up.
            let _ = std::process::Command::new("cmd")
                .args(["/C", "start", "", &target])
                .creation_flags(0x0800_0000)
                .spawn();
        }
        #[cfg(target_os = "macos")]
        let _ = std::process::Command::new("open").arg(&target).spawn();
        #[cfg(not(any(windows, target_os = "macos")))]
        let _ = std::process::Command::new("xdg-open").arg(&target).spawn();
    });
}

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
