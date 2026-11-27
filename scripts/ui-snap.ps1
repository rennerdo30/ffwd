<#
.SYNOPSIS
Fast UI iteration helper for ffwd (sub-second edit -> PNG feedback).

.EXAMPLE
pwsh scripts/ui-snap.ps1                         # build dev-ui + render all demo screens (dark + light)
pwsh scripts/ui-snap.ps1 -Only history -Audit    # render history dark+light + run programmatic audit
pwsh scripts/ui-snap.ps1 -Only sheet -Themes dark -Tokens my-tokens.toml
pwsh scripts/ui-snap.ps1 -Cranelift -Only history  # optional nightly Cranelift codegen
pwsh scripts/ui-snap.ps1 -Send "tokens my.toml; snap artifacts/ui/out.png" -Port 17321
#>
[CmdletBinding()]
param(
    [string]$OutDir = "artifacts/ui",
    [string]$Only = "",
    [ValidateSet("both", "dark", "light")]
    [string]$Themes = "both",
    [string]$Size = "1600x1000",
    [string]$Scale = "1.25",
    [string]$Tokens = "",
    [switch]$Audit,
    [switch]$NoBuild,
    [switch]$Cranelift,
    [switch]$Serve,
    [int]$Port = 17321,
    [string]$Send = ""
)

$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent

function Send-ServeCommands([int]$TcpPort, [string]$CommandBlock) {
    $client = [System.Net.Sockets.TcpClient]::new("127.0.0.1", $TcpPort)
    try {
        $stream = $client.GetStream()
        $writer = [System.IO.StreamWriter]::new($stream)
        $writer.AutoFlush = $true
        $reader = [System.IO.StreamReader]::new($stream)
        foreach ($raw in ($CommandBlock -split ';')) {
            $cmd = $raw.Trim()
            if (-not $cmd) { continue }
            $writer.WriteLine($cmd)
            $line = $reader.ReadLine()
            Write-Output $line
        }
    } finally {
        $client.Close()
    }
}

if ($Send) {
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    Send-ServeCommands -TcpPort $Port -CommandBlock $Send
    $sw.Stop()
    Write-Host ("serve roundtrip: {0} ms" -f $sw.ElapsedMilliseconds) -ForegroundColor Cyan
    exit 0
}

Push-Location $root
try {
    $meta = cargo metadata --format-version 1 --no-deps | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw "cargo metadata failed" }
    $targetDir = $meta.target_directory

    if (-not $NoBuild) {
        $swBuild = [System.Diagnostics.Stopwatch]::StartNew()
        if ($Cranelift) {
            # Cranelift runs in its own target subdir so stable dev-ui stays warm.
            $craneliftTarget = Join-Path $targetDir "cranelift"
            $env:CARGO_TARGET_DIR = $craneliftTarget
            $env:RUSTFLAGS = "-Zcodegen-backend=cranelift"
            cargo +nightly build --profile dev-ui -p ffwd --quiet
            if ($LASTEXITCODE -ne 0) { throw "cranelift build failed" }
            $exe = Join-Path $craneliftTarget "dev-ui/ffwd.exe"
        } else {
            cargo build --profile dev-ui -p ffwd --quiet
            if ($LASTEXITCODE -ne 0) { throw "dev-ui build failed" }
            $exe = Join-Path $targetDir "dev-ui/ffwd.exe"
        }
        $swBuild.Stop()
        Write-Host ("build ({0}): {1} ms" -f ($(if ($Cranelift) { "nightly cranelift dev-ui" } else { "dev-ui" }), $swBuild.ElapsedMilliseconds)) -ForegroundColor Green
    } else {
        $exe = if ($Cranelift) { Join-Path $targetDir "cranelift/dev-ui/ffwd.exe" } else { Join-Path $targetDir "dev-ui/ffwd.exe" }
    }

    if ($Serve) {
        $serveArgs = @("--serve", "$Port", "--size", $Size, "--scale", $Scale)
        if ($Tokens) { $serveArgs += @("--tokens", $Tokens) }
        & $exe @serveArgs
        exit $LASTEXITCODE
    }

    $outPath = if ([System.IO.Path]::IsPathRooted($OutDir)) { $OutDir } else { Join-Path $root $OutDir }
    $runArgs = @("--snapshot-ui", $outPath, "--size", $Size, "--scale", $Scale, "--themes", $Themes)
    if ($Only) { $runArgs += @("--only", $Only) }
    if ($Tokens) { $runArgs += @("--tokens", $Tokens) }
    if ($Audit) { $runArgs += @("--audit", (Join-Path $outPath "audit.json")) }

    $swRun = [System.Diagnostics.Stopwatch]::StartNew()
    & $exe @runArgs
    $code = $LASTEXITCODE
    $swRun.Stop()
    Write-Host ("render: {0} ms -> {1}" -f $swRun.ElapsedMilliseconds, $outPath) -ForegroundColor Cyan
    exit $code
} finally {
    Pop-Location
}
