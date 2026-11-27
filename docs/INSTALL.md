# Installing ffwd

Download from [Releases](https://github.com/rennerdo30/ffwd/releases).

## Windows

- **Setup:** run `ffwd-<version>-setup.exe`.
  - It installs per user to `%LOCALAPPDATA%\Programs\ffwd` without admin rights, and adds a Start menu entry and an Installed apps entry.
  - It does not change PATH or file types.
  - The installed copy updates itself.
- **Silent install:** `ffwd-<version>-setup.exe /S`.
- **Portable:** unpack `ffwd-<version>-windows-x64.zip` anywhere and run `ffwd.exe`. A portable copy reports updates but does not install them.
- **Uninstall:** Settings → Apps → Installed apps → ffwd, or `"%LOCALAPPDATA%\Programs\ffwd\uninstall.exe" /S`. Your settings in `%APPDATA%\ffwd` stay.

Builds are not signed yet, so SmartScreen warns on the first run (More info → Run anyway).

## macOS and Linux

Unpack `ffwd-<version>-macos-aarch64.tar.gz` or `ffwd-<version>-linux-x86_64.tar.gz` and run `ffwd`.
- On macOS, the first start of an unsigned app needs right-click → Open.
- These builds report updates with a link to the release page.

ffwd needs Git (`git` on PATH).

## From crates.io

With Rust 1.100 or newer, on any of the three systems:

```sh
cargo install --locked ffwd@<version>   # e.g. ffwd@0.2.0-beta.12
```

- While ffwd is in beta, name the version (the newest is on [crates.io](https://crates.io/crates/ffwd)); `cargo install` skips prereleases otherwise.
- A copy installed this way names the `cargo install` command when a new version is out.

## Verifying a download

`SHA256SUMS.txt` in each release lists every package:

```sh
sha256sum -c SHA256SUMS.txt --ignore-missing          # Linux
shasum -a 256 -c SHA256SUMS.txt --ignore-missing      # macOS
```

On Windows, compare it with `Get-FileHash .\ffwd-<version>-setup.exe`.
