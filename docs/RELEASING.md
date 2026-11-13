# Releasing ffwd

Every release needs the owner's yes. CI builds and tests the packages and prepares a **draft** GitHub release. Publishing that draft is the owner's yes; nothing in CI publishes on its own.

## Versions

There is one version for the whole workspace: `version` under `[workspace.package]` in `Cargo.toml`, following SemVer.
- Prereleases look like `0.2.0-beta.1`. Their tags have a `-` and become GitHub prereleases.
- A stable `0.2.0` ranks above every `0.2.0-…`.

## Cutting a release

1. **Prepare the commit on `main`:**
   - set the workspace version;
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

The setup's file name `ffwd-<version>-setup.exe` and `SHA256SUMS.txt` are the contract with the in-app updater; keep both names.

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
