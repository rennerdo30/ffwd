# 0002: Agent workspaces (proposed)

**Status:** proposed on 2026-11-13. It grows out of feedback from a friend of the owner (Pair Desk FF-22) and needs the owner's direction before any of it is built.

## Idea

ffwd already holds every repository you work on, one tab each. Each tab can also host a **coding agent session** that works in a sandbox:

1. **Start:** you open a repository and write a prompt, in a panel next to Local Changes.
2. **Isolation:** the agent works in its **own Git worktree** of that repository, on a new branch, never in your checkout.
3. **Sandbox, default deny:**
   - The process can see only the worktree, plus folders you add with one click. Each grant applies from the next prompt on.
   - Linux uses bubblewrap. Windows uses the same bubblewrap policy inside WSL. macOS uses its sandbox profile.
   - Network access is off unless the session needs the AI endpoint.
4. **Parallel work:** you switch to another tab while it works. When it finishes, the tab gets a badge and the OS shows a notification.
5. **Review, not trust:**
   - The result is a branch: normal commits in ffwd's history, with ffwd's diff view.
   - You merge, cherry-pick, edit or discard it. Nothing reaches your branch without your click.

This turns ffwd's AI plan from "suggest a commit message" into "delegate a task and review it like a pull request". The review is the part a Git client does best, and it is what ffwd is built for.

## What it needs first

- **The operations layer must be trustworthy (FF-13).** The friend's warning applies to everything ffwd does: every Git command has to do exactly what it says and handle every failure cleanly. That comes before any agent feature. See the matrix below.
- **Worktrees as a first-class view** (the full feature set, M2).
- **Local Changes and a commit view (FF-14),** to review an agent's work.

## Git operations: failures ffwd must handle

Each failure must be detected before running or recognised from Git's output, explained in one plain sentence with the way out, and covered by an integration test against a throwaway repository.

| Situation | What ffwd does |
|---|---|
| No upstream on push or pull | Offer to set `origin/<branch>`, naming it |
| Push rejected (non-fast-forward) | Explain that the remote has new commits; offer fetch and then rebase or merge, never force |
| Pull would merge diverged histories | Stop and ask: rebase or merge, showing how many commits are on each side |
| Uncommitted changes would be overwritten (checkout, pull, rebase) | Name the files; offer to stash and re-apply |
| Merge or rebase conflict | Open the conflict view; keep abort one click away |
| An operation is already in progress (rebase, merge, cherry-pick, bisect) | Show the banner with continue, skip and abort |
| Index lock left behind (`index.lock`) | Say so; offer to remove the lock only if no Git process is running |
| Authentication failed or timed out | Say which remote failed and why; never loop on prompts |
| Detached HEAD | Show it; offer to create a branch before committing |
| Hooks failing (pre-commit, pre-push) | Show the hook's output; keep the commit message |
| Shallow clone, LFS missing, submodule out of sync | Name the cause; offer the specific fix |

Every operation shows the exact command and its full output in an operation log. Destructive operations take a backup ref first, so they can be undone (AGENTS.md rule 8).

## The Pull button

Pull merges by default when the histories have diverged. That silently creates merge commits, or a conflict, from a single click, which is why many people avoid it. Proposal:

- **Pull** fast-forwards only.
- When a fast-forward is impossible, ffwd stops and asks **rebase or merge**, showing the commits on each side.
- An explicit `pull.rebase` in the user's Git config is respected.

This changes a default the user sees, so it needs the owner's confirmation before it is built.
