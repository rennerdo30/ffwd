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

## Verifying a download

`SHA256SUMS.txt` in each release lists every package:

```sh
sha256sum -c SHA256SUMS.txt --ignore-missing          # Linux
shasum -a 256 -c SHA256SUMS.txt --ignore-missing      # macOS
```

On Windows, compare it with `Get-FileHash .\ffwd-<version>-setup.exe`.
