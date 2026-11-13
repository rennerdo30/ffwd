# Agent workspaces (experimental)

> **Experimental.** This feature is off by default, and its behaviour and settings may still change. It runs programs you configure on your machine. Read "What the sandbox protects" before you turn it on.

An agent run hands one task to a coding agent of your choice: Aider, Claude Code, Codex CLI, a local model behind a script, or anything else with a command line. ffwd gives the agent its own worktree on a new branch and starts it inside a sandbox. When the agent ends, ffwd commits what it changed to that branch. You then review the branch like a pull request and either merge it or discard it.

The design is in [decisions/0002-agent-workspaces.md](decisions/0002-agent-workspaces.md).

## Using it

1. Turn it on: open **Settings → Agents**, set **Agents** to *On (experimental)*, then pick an **Agent** (Claude, Codex, agy, opencode or Muse) or choose *Custom* and set an **Agent command**. **Applies to** decides whether a change is for every repository or only for the current one (see "Per-repository settings").
2. Press **Ctrl+Shift+A**, or choose *Agents (experimental)…* in the `Ctrl+K` palette, to open the agents panel of the current repository.
3. Type a task and press **Enter**. The run appears in the list with its live log. The tab shows a dot while an agent runs. When the run ends, the dot turns green for success or red for failure until you open the panel. The status bar also says when a run has ended.
4. Review it:
   - **Enter** (or *Show branch*) selects the branch in the history, where the diff view shows the agent's commits.
   - **m** merges the branch into your current branch with `git merge --no-ff`, after you confirm.
   - **d** discards the run, after you confirm: ffwd removes the worktree and deletes the branch, then shows the `git branch …` command that restores it.
   - **c** cancels a running agent. Whatever it changed so far is still committed to its branch.
   - **o** opens the worktree folder and **l** opens the full log.

Runs survive a restart. Their worktrees and branches stay where they are, and the panel lists them again. A run that was active when ffwd closed comes back as *interrupted*. Whatever it changed is still in its worktree, uncommitted. Commit it there by hand, or discard the run.

## What a run does

1. It creates `git worktree add -b <branch_prefix><id> <worktree_root>/<id> <base>`. Your checkout is never touched.
2. It writes the prompt to `prompt.md` in the run folder (`<worktree_root>/.runs/<id>/`).
3. It runs the **setup script**, then the **agent**, then the **after script**, each in the worktree, inside the sandbox and under the time limit.
4. If `auto_commit` is on, it runs `git add -A` and `git commit` in the worktree. This happens outside the sandbox, with your identity and your hooks.
5. It counts the commits and the diffstat against the base.
6. It runs the **on-done script** on your machine (outside the sandbox), for example to send a notification.

Everything runs on worker threads, so the window never waits for an agent. The log is kept in memory (`log_lines`) and in `log.txt` in the run folder, capped at 8 MB.

## What the sandbox protects

The idea is to deny everything by default and grant access explicitly.

| | bubblewrap (Linux) and WSL + bubblewrap (Windows) | macOS sandbox |
|---|---|---|
| System programs and libraries | read-only (`/usr`, `/bin`, `/lib*`, `/etc`, `/opt`, `/nix`) | readable |
| Your home folder | an empty home in `/tmp` (`home_access = "private"`) | not readable (`private`) |
| The worktree | read and write | read and write |
| The repository's Git data | read-only (`git_access = "read"`) | read-only |
| `read_paths` and `write_paths` | as granted | as granted |
| Network | on or off (`network`) | on or off |
| Processes | own PID namespace, ends with ffwd | — |

What the sandbox does **not** do:

- With `network = true` an agent can send anything it can read to the internet. That includes the repository it works on.
- With `git_access = "write"` the agent shares the repository's object store and refs. It could then change other branches. With the default `"read"` it cannot.
- With `home_access = "read"` it can read your keys and tokens. Prefer granting only the folder the agent needs (for example `~/.config/<tool>`) in `read_paths` or `write_paths`.
- `sandbox = "none"` gives the agent all your permissions. ffwd refuses to run that way unless you also set `allow_unsandboxed = true`.
- The sandbox does not protect against a malicious agent binary that exploits the kernel or the sandbox itself.

### Windows: where does the agent run?

On Windows the default sandbox is WSL + bubblewrap, which means **the agent runs inside WSL, not on Windows**. A tool installed on Windows (`claude.exe`, `codex`, `agy`, `opencode`, `muse`) does not exist there, and a Windows program seen through `/mnt/c` cannot run inside the sandbox. The agents panel's top line always says where the agent runs ("runs inside WSL (Debian)", or "runs on Windows").

Before it touches the repository, every run checks that the agent and bubblewrap exist in that place. If either is missing, the run stops with the fix. You have two choices:

1. **Sandboxed:** install the tool inside WSL and log in there once. Inside WSL each tool has its own Linux login and config (for example `~/.claude` in the Linux home), separate from Windows. You also need `sudo apt install bubblewrap` there.
2. **Windows version, not sandboxed:** set Sandbox to *None* and Allow no sandbox to *Yes*. The Windows tool then runs with all your permissions. ffwd still gives it its own worktree and branch, and still applies the time limit and the review step.

Choice 2 can apply to one repository only (see "Per-repository settings").

A `subst` drive (for example `E:` standing for `D:\`) is not mounted in WSL; map it with `wsl_path_map = "E:\=>/mnt/d"`.

ffwd stops only the processes it started itself. A cancel or time-out kills that process tree (`taskkill /T` on Windows, the process group on Unix), and closing ffwd stops every run it started.

## Agents (presets)

**Settings → Agents → Agent** picks a built-in tool. A preset supplies three things:

- the non-interactive command line;
- the tool's login and config folders, granted writable (`preset_grants`);
- the names of its API-key variables, passed through. ffwd never reads their values.

Inside WSL the `~` folders are the Linux ones.

| Preset | Command | Folders granted | Key variables passed |
|---|---|---|---|
| Claude (`claude`) | `claude -p {prompt} --permission-mode acceptEdits` | `~/.claude`, `~/.claude.json` | `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_BASE_URL`, `ANTHROPIC_MODEL` |
| Codex (`codex`) | `codex exec -s workspace-write {prompt}` | `~/.codex` | `OPENAI_API_KEY`, `OPENAI_BASE_URL`, `CODEX_HOME` |
| agy (`antigravity`) | `agy -p {prompt} --mode accept-edits` | `~/.gemini` | `GEMINI_API_KEY`, `GOOGLE_API_KEY`, `GOOGLE_CLOUD_PROJECT` |
| opencode (`opencode`) | `opencode run --standalone --auto {prompt}` | `~/.config/opencode`, `~/.local/share/opencode`, `~/.local/state/opencode`, `~/.cache/opencode` | `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `GEMINI_API_KEY`, `OPENROUTER_API_KEY` |
| Muse (`muse`) | `muse exec --prompt-file {prompt_file} --worktree off --approval-mode never` | `~/.config/muse` | — |

The flags were checked against each tool's `--help` on 2026-11-13 with these versions: Claude Code 2.1.322, Codex CLI 0.167.2, agy 1.5.8, opencode 2.2.7, Muse Code 1.4.14. A few flags are worth explaining:

- opencode's `--standalone` keeps its server inside the sandbox; without it, opencode talks to a background service that runs outside the sandbox.
- Muse's `--worktree off` stops Muse from making its own worktree, because ffwd already made one.

Tools change their flags. If a preset breaks, set **Agent command** yourself; it always wins over the preset's. `preset_grants = false` drops the preset's folders and variables.

## Per-repository settings

Every `[agents]` setting can be set for one repository. Set **Settings → Agents → Applies to** to *This repository*: changes then go into that repository's own file and override the global value there. Rows the repository overrides carry a dot, and `Delete` returns a row to the global value. In *All repositories* the dot is dim and the hint says the repository overrides it. The agents panel's top line counts the overrides.

The file is `<ffwd config folder>/repos/<name>-<hash>.toml`, in the same `[agents]` format as `settings.toml`. It can be edited by hand. Overrides are **never read from the repository itself**: a cloned repository must not be able to set a command, a script or the sandbox, because that would let any repository run code on your machine.

Some uses:

- Turn agents on for some repositories only (`enabled`).
- Use another agent per project (`preset`, `command`).
- Use a project's own `setup_script` and `after_script`.
- Grant folders for one project only (`read_paths`, `write_paths`).
- Give a long-running project a longer time limit.

## Settings

All settings live under `[agents]` in `settings.toml`. The most important ones are also in **Settings → Agents**, and **Open settings.toml** there shows the file. Lists of folders use `;` as the separator. `env` and `wsl_path_map` use `|`, because their values may contain `;`.

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `false` | Turns the feature on. |
| `preset` | `custom` | `claude`, `codex`, `antigravity` (or `agy`), `opencode`, `muse` or `custom`. See "Agents (presets)". |
| `preset_grants` | `true` | Adds the preset's config folders and key variable names. |
| `preset_access` | — | The access each of the preset's own paths gets, `path=off|read|write` with `;` between: for example `~/.claude=read; ~/.claude.json=off`. A path not named stays writable. |
| `command` | the preset's | The agent's command line. See "Command templates" below. |
| `prompt_stdin` | `false` | Also writes the prompt to the agent's standard input. |
| `prompt_template` | `"{prompt}"` | Wraps your text, for example `"Work only in src/. Task: {prompt}"`. It may use `{prompt}`, `{branch}`, `{base}`, `{repo}` and `{worktree}`. |
| `prompt_template_file` | — | A file whose text is the template (it wins over `prompt_template`). This is how you inject a standing context, rules or a checklist into every run. |
| `sandbox` | `"auto"` | `auto`, `bwrap`, `wsl`, `macos`, `custom` or `none`. `auto` means bwrap on Linux, WSL on Windows and the macOS sandbox on macOS. |
| `sandbox_command` | — | The wrapper for `custom`. `{command}`, as its own argument, stands for the agent's whole command line, for example `firejail --private={worktree} {command}` or `docker run --rm -v {worktree}:/w -w /w my-agent-image {command}`. |
| `allow_unsandboxed` | `false` | Required for `sandbox = "none"`. |
| `network` | `true` | Network inside the sandbox. Agents that call a hosted model need it. |
| `git_access` | `"read"` | `read`: the agent edits files and ffwd commits them. `write`: the agent may commit (see the caveat above). |
| `home_access` | `"private"` | `private`, `read` or `write`. |
| `read_paths` | — | Extra folders or files the agent may read, `;` between them. `~/…` is the home where the agent runs. |
| `write_paths` | — | Extra folders or files the agent may write. `~/…` as above. |
| `env_pass` | a short list | Host variables passed through by name (`PATH`, `HOME`, `LANG`, `TEMP`, …). Secrets are passed only if you name them here. |
| `env_inherit` | `false` | Passes the whole host environment (`none` and `custom` only). |
| `env` | — | Extra variables, `KEY=value|KEY2=value`. The values never appear on a command line. |
| `worktree_root` | next to the repository | Where worktrees go. The default is `<repository's parent>/.ffwd-agents/<repository>`. When set, ffwd uses `<worktree_root>/<repository>`. |
| `branch_prefix` | `"agent/"` | The prefix of every agent branch. |
| `base` | `"HEAD"` | What the branch starts from (any commit, branch or tag). |
| `timeout_minutes` | `60` | The agent is stopped after this. Range 1 to 1440. |
| `max_parallel` | `2` | Runs at the same time, over all tabs. |
| `setup_script` | — | Runs in the worktree before the agent (for example `npm ci`, or copying a `.env`). If it fails, the agent does not start. |
| `after_script` | — | Runs after the agent and before ffwd commits (for example the tests, or a formatter). It gets `FFWD_AGENT_EXIT`. It does not run after a cancel. |
| `on_done_script` | — | Runs on your machine when a run ends, for example `notify-send`, `terminal-notifier` or a chat webhook. It gets `FFWD_AGENT_STATUS` and `FFWD_AGENT_COMMITS`. |
| `script_shell` | `sh -c` / `cmd /C` | The shell for scripts, for example `pwsh -NoProfile -Command`. |
| `scripts_in_sandbox` | `true` | Setup and after scripts run in the agent's sandbox. |
| `script_timeout_minutes` | `15` | The time limit for each script. |
| `auto_commit` | `true` | Commits what the agent left. |
| `commit_message` | `"Agent: {title}"` | It may use `{title}` (the first line of the prompt), `{prompt}`, `{branch}` and `{run_id}`. |
| `commit_no_verify` | `false` | Skips your commit hooks for that commit. |
| `wsl_distro` | default | `wsl -d <name>`. |
| `wsl_path_map` | — | Windows prefixes that WSL does not mount as `/mnt/<letter>`, for example a `subst` drive: `E:\=>/mnt/d`. |
| `bwrap_path` | `"bwrap"` | The bubblewrap program. |
| `log_lines` | `5000` | Log lines kept in memory per run. |

Values ffwd cannot read are named in the agents panel, and the default is used instead.

### Command templates

ffwd splits the command into arguments **first** and only then fills in the placeholders. A prompt with quotes, `;`, `$( )` or `--options` therefore stays exactly one argument and can never become a command or an option. Quoting works like a shell: `'…'` is literal, and `"…"` allows `\"`. A backslash outside quotes is an ordinary character, so Windows paths need no escaping.

The placeholders are `{prompt}`, `{prompt_file}`, `{worktree}`, `{branch}`, `{base}`, `{repo}` and `{run_id}`. Inside WSL they hold the Linux paths. Write `{{` and `}}` for literal braces.

Every process also gets these variables: `FFWD_AGENT_ID`, `FFWD_AGENT_BRANCH`, `FFWD_AGENT_BASE`, `FFWD_AGENT_REPO`, `FFWD_AGENT_WORKTREE`, `FFWD_AGENT_RUN_DIR`, `FFWD_AGENT_PROMPT_FILE` and `FFWD_AGENT_TITLE`.

These examples are a starting point; check your tool's own documentation for its current flags:

```toml
[agents]
enabled = true
# Aider with a local model through Ollama
command = "aider --yes-always --model ollama_chat/qwen2.5-coder:14b --message-file {prompt_file}"
env = "OLLAMA_API_BASE=http://127.0.0.1:11434"

# Claude Code, non-interactive, allowed to edit files
# command = "claude -p {prompt} --permission-mode acceptEdits"
# write_paths = "/home/me/.claude"

# Codex CLI
# command = "codex exec --full-auto {prompt}"

setup_script = "npm ci"
after_script = "npm test"
on_done_script = "notify-send \"ffwd\" \"$FFWD_AGENT_TITLE: $FFWD_AGENT_STATUS\""
```

## Not yet

Pair Desk tracks these: FF-27 (checking real runs in each sandbox) and FF-28 (the gaps and ideas below).

- Native sandboxes on Windows (AppContainer or a restricted token) are not there yet; Windows uses WSL.
- Network filtering per host, beyond on and off.
- An OS notification without a script.
- A conflict view after a merge that stops on conflicts. The panel says how to abort it.
