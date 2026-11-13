# Writes assets/logo/ffwd.ico (16, 24, 32, 48, 64 and 256 px, PNG entries)
# from the icon ffwd draws itself (`ffwd --icon-png <file> <size>`), so the
# installer icon always matches the window icon and the SVG logo.
#   scripts\make-icon.ps1
[CmdletBinding()]
param([string]$Output = (Join-Path (Split-Path -Parent $PSScriptRoot) 'assets/logo/ffwd.ico'))
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$sizes = 16, 24, 32, 48, 64, 256
$tmp = Join-Path ([IO.Path]::GetTempPath()) ('ffwd-icon-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
    $entries = foreach ($size in $sizes) {
        $png = Join-Path $tmp "$size.png"
        # cargo waits for ffwd (a GUI-subsystem exe in release builds).
        & cargo run --release --quiet -j 8 -p ffwd --manifest-path (Join-Path $root 'Cargo.toml') -- --icon-png $png $size
        if ($LASTEXITCODE -or !(Test-Path -LiteralPath $png)) { throw "ffwd --icon-png failed for $size px" }
        , [IO.File]::ReadAllBytes($png)
    }
    $ms = [IO.MemoryStream]::new()
    $w = [IO.BinaryWriter]::new($ms)
    $w.Write([uint16]0); $w.Write([uint16]1); $w.Write([uint16]$sizes.Count)
    $offset = 6 + 16 * $sizes.Count
    for ($i = 0; $i -lt $sizes.Count; $i++) {
        $dim = if ($sizes[$i] -ge 256) { 0 } else { $sizes[$i] }
        $w.Write([byte]$dim); $w.Write([byte]$dim); $w.Write([byte]0); $w.Write([byte]0)
        $w.Write([uint16]1); $w.Write([uint16]32)
        $w.Write([uint32]$entries[$i].Length); $w.Write([uint32]$offset)
        $offset += $entries[$i].Length
    }
    foreach ($bytes in $entries) { $w.Write($bytes) }
    $w.Flush()
    [IO.File]::WriteAllBytes($Output, $ms.ToArray())
    Write-Host "Wrote $Output ($($ms.Length) bytes)"
} finally {
    Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
}
