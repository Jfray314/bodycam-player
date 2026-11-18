# Installs UE4SS (experimental build, UE 5.5 support) into your Bodycam install and enables the BodycamProbe mod.
# Needed once, to produce extracted/Bodycam.usmap (UE4SS keybind Ctrl+Numpad6 = "Dump USMAP") that tools/setup.py asks for.
# This is the ONLY thing in this repo that writes into the game folder, and only when you run it yourself.
# Run:   powershell -ExecutionPolicy Bypass -File tools\ue4ss_probe\install.ps1 [-Game "<Bodycam install dir>"]
# Undo:  powershell -ExecutionPolicy Bypass -File tools\ue4ss_probe\install.ps1 -Uninstall
param([string]$Game = $(if ($env:BODYCAM_GAME) { $env:BODYCAM_GAME } else { "C:\Program Files (x86)\Steam\steamapps\common\Bodycam" }), [switch]$Uninstall)
$ErrorActionPreference = "Stop"
$win64 = Join-Path $Game "Bodycam\Binaries\Win64"
if (-not (Test-Path (Join-Path $win64 "Bodycam-Win64-Shipping.exe"))) { throw "Bodycam not found at $win64 (pass -Game <install dir>)" }

if ($Uninstall) {
    foreach ($p in "dwmapi.dll", "ue4ss", "UE4SS.dll", "UE4SS-settings.ini", "Mods", "UE4SS.log") {
        $f = Join-Path $win64 $p
        if (Test-Path $f) { Remove-Item $f -Recurse -Force; Write-Host "removed $f" }
    }
    return
}

# 1. UE4SS: latest experimental release (the stable 3.0.1 doesn't support UE 5.5).
$rel = Invoke-RestMethod "https://api.github.com/repos/UE4SS-RE/RE-UE4SS/releases/tags/experimental-latest" -Headers @{ "User-Agent" = "bodycam-probe" }
$asset = $rel.assets | Where-Object { $_.name -match '^UE4SS_v.*\.zip$' } | Select-Object -First 1
if (-not $asset) { throw "no UE4SS_v*.zip asset in experimental-latest" }
$zip = Join-Path $env:TEMP $asset.name
Write-Host "downloading $($asset.name) ($([math]::Round($asset.size / 1MB, 1)) MB)"
Invoke-WebRequest $asset.browser_download_url -OutFile $zip
Expand-Archive $zip -DestinationPath $win64 -Force

# 2. Install the mod next to UE4SS's own Mods folder (new layout: Win64\ue4ss\Mods, old: Win64\Mods).
$mods = @((Join-Path $win64 "ue4ss\Mods"), (Join-Path $win64 "Mods")) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $mods) { throw "UE4SS Mods folder not found after extracting" }
Copy-Item (Join-Path $PSScriptRoot "BodycamProbe") $mods -Recurse -Force
$list = Join-Path $mods "mods.txt"
if (Test-Path $list) {
    $lines = Get-Content $list | Where-Object { $_ -notmatch '^\s*BodycamProbe\s*:' }
    Set-Content $list (@("BodycamProbe : 1") + $lines) -Encoding ascii
}

# 3. Recording output folder (the installed copy of main.lua writes there).
$probe = Join-Path (Resolve-Path (Join-Path $PSScriptRoot "..\..")) "extracted\probe"
New-Item -ItemType Directory -Force $probe | Out-Null
$lua = Join-Path $mods "BodycamProbe\Scripts\main.lua"
$probeFwd = $probe.Replace("\", "/")
(Get-Content $lua) -replace '^local OUT = .*$', "local OUT = `"$probeFwd/`"" | Set-Content $lua
Write-Host "UE4SS + BodycamProbe installed in $win64"
Write-Host "Start Bodycam and spawn in; press Ctrl+Numpad6 (UE4SS 'Dump USMAP'), then copy Mappings.usmap from the UE4SS folder to extracted\Bodycam.usmap."
Write-Host "Probe keys: F8 = start/stop recording, F9 = marker, F7 = property dump."
