# Releasing ffwd

Every release needs the owner's yes. CI builds and tests the packages and prepares a **draft** GitHub release. Publishing that draft is the owner's yes; nothing in CI publishes on its own.

## Versions

There is one version for the whole workspace: `version` under `[workspace.package]` in `Cargo.toml`, following SemVer. The four internal crates under `[workspace.dependencies]` (`ffwd-core`, `ffwd-ai`, `ffwd-update` and `ffwd-agent`) pin that same version exactly (`version = "=X.Y.Z"`). A bump changes all five lines, and cargo refuses to build if one is missed.

**ffwd-render has its own version** (in `crates/ffwd-render/Cargo.toml`), because other projects depend on it from crates.io:
- it follows SemVer for its public API: a breaking change bumps the minor version while it is `0.x`;
- the workspace pin for it is an ordinary requirement (`version = "0.1.0"`);
- its releases are tagged `ffwd-render-vX.Y.Z`.
- Prereleases look like `0.2.0-beta.1`. Their tags have a `-` and become GitHub prereleases.
- A stable `0.2.0` ranks above every `0.2.0-…`.

## Cutting a release

1. **Prepare the commit on `main`:**
   - set the workspace version and the four internal `[workspace.dependencies]` pins;
   - if ffwd-render changed since its last release, bump its own version too;
   - move the `## Unreleased` notes in [`CHANGELOG.md`](../CHANGELOG.md) into a `## X.Y.Z` section;
   - commit, and let CI run.
2. **Tag that commit and push the tag:**
   ```sh
   git tag -a v0.2.0-beta.1 -m "ffwd 0.2.0-beta.1"
   git push origin refs/tags/v0.2.0-beta.1
   ```
3. **The release workflow runs** ([`.github/workflows/release.yml`](../.github/workflows/release.yml)):
   - It checks that the tag matches the workspace version.
   - Windows:
     - builds `ffwd-<version>-setup.exe` (per-user NSIS 3.13 setup) and `ffwd-<version>-windows-x64.zip`;
     - runs the installer lifecycle test on the disposable runner: install, reinstall, a running ffwd blocks setup, uninstall leaves nothing.
   - macOS (Apple silicon) and Linux (x86_64): builds `ffwd-<version>-<os>-<arch>.tar.gz`.
   - Every package carries `LICENSE.txt` and `THIRD-PARTY-NOTICES.txt` (cargo-about, `about.toml`).
   - It writes one `SHA256SUMS.txt` over all packages and creates a **draft** release with the changelog section as notes.
   - Rerunning it (Actions → release → Run workflow, with the tag) refreshes a draft and never touches a published release.
4. **The owner reviews the draft and publishes it.** Only then do installed copies see the update.
5. **crates.io** (optional, with the owner's yes; below).

The setup's file name `ffwd-<version>-setup.exe` and `SHA256SUMS.txt` are the contract with the in-app updater; keep both names.

## crates.io

`cargo install ffwd` builds ffwd from crates.io, so the app's crates have to be there too:
- `ffwd-render` is a library for other projects as well, with its own version;
- `ffwd-core`, `ffwd-ai`, `ffwd-update` and `ffwd-agent` are marked "Internal to ffwd: no stable API".

- **Every change is packaged in CI.** The `crates.io packages` job runs `cargo package --workspace`. It packs all six crates, builds them from the packed sources alone and checks that each crate's `LICENSE` copy matches the root one. Nothing is uploaded.
- **Publishing is manual,** through Actions → publish-crate → Run workflow, with a tag on `main`:
  - `vX.Y.Z`: a released ffwd version. Its GitHub release must already be published. This publishes the app and the four internal crates, and ffwd-render if its version is not on crates.io yet.
  - `ffwd-render-vX.Y.Z`: publishes ffwd-render alone. The tag must match its version.
- **The two jobs:**
  1. `verify` runs without a token. It checks the tag, then packages and builds the chosen crates from their packed sources.
  2. `publish` runs in the `crates-io` environment. It checks out the exact commit `verify` checked and runs `cargo publish --no-verify` for the same crates. That is the only step that sees the token.
- **One-time setup by the owner:**
  - a crates.io API token with the `publish-new` and `publish-update` scopes, limited to the crate pattern `ffwd*`;
  - stored as the `CARGO_REGISTRY_TOKEN` secret of a `crates-io` environment, with the owner as required reviewer.
- **Prereleases:** `cargo install ffwd` skips prereleases. Until a stable version is out, users name one: `cargo install --locked ffwd@0.2.0-beta.10`.
- **A version is permanent.** crates.io can yank a version but never delete it or reuse its number; a fix is a new version.
- **Package contents:**
  - each crate's `include` list: its sources and `LICENSE`;
  - for the app, also `build.rs`, `assets/ffwd.ico` (the Windows icon) and the root `README.md`.
  Paths outside a crate's folder do not exist in its package.
- **Updates:** a copy built by `cargo install` (one in Cargo's `bin` folder) names the `cargo install` command for a new version instead of the release page.

## Updates in the app

Updates need two things: a **published** release (drafts are invisible) and a copy **installed by the setup**. Development builds (`cargo run`, `scripts\run.bat`) never update; `Ctrl+K` → **Check for updates** says so, and in an installed copy it checks right away.

- An installed copy checks once after its first frame, at most once a day. The check runs on a worker thread.
- A found update shows in the status bar. `Ctrl+Shift+U` (or "Install update" in `Ctrl+K`) installs it:
  1. ffwd downloads the setup and `SHA256SUMS.txt` into a fresh temporary folder;
  2. it checks the setup's SHA-256 and runs the setup only on a match, with `/S /UPDATE`;
  3. ffwd closes, and the setup waits for it, installs and starts ffwd again.
- **Channels:** a prerelease build follows **beta** (newest published release, prerelease or stable); a stable build follows **stable**.
- **Settings** (`%APPDATA%\ffwd\settings.toml`):
  ```toml
  [update]
  check = true          # false: no checks
  channel = "stable"    # or "beta"; omitted: follows the running version
  ```
- **No check at all** in debug builds, with `FFWD_NO_UPDATE` set, in `--snapshot` and `--smoke` runs.
- **Report only:** portable copies (zip, tar.gz) and copies outside `%LOCALAPPDATA%\Programs\ffwd` only name the release page.
- **Fetched URLs:** only GitHub's API for `rennerdo30/ffwd` and its release downloads, over HTTPS.
  - Windows: WinHTTP, so the system proxy settings apply.
  - Elsewhere: the system `curl`.
  - Responses are size-capped and time out after 60 s.

## What protects an update

- **The checksum protects integrity, not authenticity.** It comes from the same release as the setup.
- **The setup is not Authenticode-signed yet**, so Windows SmartScreen warns on the first run. Signing is a separate owner decision.
- **Nothing runs elevated.** The setup installs per user.

## Building the packages locally

`scripts\package.bat` builds the release binary, the setup, the zip and `SHA256SUMS.txt` into `artifacts\installer`.
- **NSIS:** it uses the portable NSIS 3.13 at `E:/Development/_cache/tools/nsis-3.13/`; `scripts/fetch-nsis.ps1` downloads the pinned ZIP elsewhere.
- **Notices:** without cargo-about it writes a placeholder notice. Release builds always generate the real one in CI.
- **Never run `scripts/test-installer.ps1` on a real PC.** It installs and uninstalls for the current user, and refuses to run outside GitHub Actions.
