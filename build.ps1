# Build RE-Formatter and embed the icon. The finished exe stays in
# target\release\usb-reformat.exe (it is NOT copied anywhere else).
#
# IMPORTANT: `cargo build` regenerates the exe WITHOUT the embedded icon, so the
# `seticon` step must run after EVERY build. Use this script instead of calling
# `cargo build` directly.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"

$manifest = Join-Path $here 'Cargo.toml'
$outExe   = Join-Path $here 'target\release\usb-reformat.exe'
$seticon  = Join-Path $here 'target\release\seticon.exe'
$icon     = Join-Path $here 'ICON1.ico'
$appMani  = Join-Path $here 'manifest.xml'
$musicDir = Join-Path $here 'music'

Get-Process -Name usb-reformat -ErrorAction SilentlyContinue |
    Stop-Process -Force -ErrorAction SilentlyContinue

cargo build --release --manifest-path $manifest
if ($LASTEXITCODE -ne 0) { throw "cargo build failed (exit $LASTEXITCODE)" }

# Icon + application manifest (requireAdministrator / common controls v6) +
# the chiptunes as RCDATA resources, played from memory at runtime.
& $seticon $outExe $icon $appMani $musicDir | Out-Null
if ($LASTEXITCODE -ne 0) { throw "seticon failed (exit $LASTEXITCODE)" }

Write-Host "OK - built, icon + manifest + music embedded: $outExe"
