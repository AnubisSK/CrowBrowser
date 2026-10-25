# CrowBrowser Windows release: icon -> release binary -> Inno Setup installer in dist\.
# Idempotent: re-running rebuilds what changed and overwrites the installer of this version.
#
#   powershell -ExecutionPolicy Bypass -File scripts\build_windows_installer.ps1 [-SkipBuild] [-RegenerateIcon]
#
# Needs: Rust (x86_64-pc-windows-msvc), Visual Studio Build Tools (MSVC + Windows SDK for rc.exe),
# Inno Setup 6 (https://jrsoftware.org/isdl.php; or set $env:ISCC to ISCC.exe).
[CmdletBinding()]
param(
    [switch]$SkipBuild,
    [switch]$RegenerateIcon
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$Root = Split-Path -Parent $PSScriptRoot
$Target = 'x86_64-pc-windows-msvc'
$Ico = Join-Path $Root 'assets\crow.ico'
$Png = Join-Path $Root 'assets\logo-1024.png'
$Exe = Join-Path $Root "target\$Target\release\crow-browser.exe"
$Iss = Join-Path $Root 'package\windows\installer.iss'
$Dist = Join-Path $Root 'dist'

function Step($n, $text) { Write-Host "[$n/6] $text" -ForegroundColor Cyan }

# Multi-size icon (16-256 px, PNG-compressed entries, Vista+) from the 1024 px logo. ImageMagick
# when installed, else System.Drawing (part of Windows PowerShell).
function New-CrowIcon([string]$src, [string]$dst) {
    $sizes = 16, 20, 24, 32, 40, 48, 64, 96, 128, 256
    $magick = Get-Command magick -ErrorAction SilentlyContinue
    if ($magick) {
        & $magick.Source $src -background none -define "icon:auto-resize=$($sizes -join ',')" $dst
        if ($LASTEXITCODE -ne 0) { throw 'ImageMagick could not write the icon' }
        return
    }
    Add-Type -AssemblyName System.Drawing
    $source = [System.Drawing.Image]::FromFile($src)
    try {
        $images = foreach ($s in $sizes) {
            $bmp = New-Object System.Drawing.Bitmap $s, $s, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
            $g = [System.Drawing.Graphics]::FromImage($bmp)
            $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
            $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
            $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
            $g.DrawImage($source, 0, 0, $s, $s)
            $g.Dispose()
            $ms = New-Object System.IO.MemoryStream
            $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
            $bmp.Dispose()
            , @($s, $ms.ToArray())
        }
    } finally {
        $source.Dispose()
    }
    $out = New-Object System.IO.MemoryStream
    $w = New-Object System.IO.BinaryWriter $out
    $w.Write([uint16]0); $w.Write([uint16]1); $w.Write([uint16]$images.Count)   # ICONDIR
    $offset = 6 + 16 * $images.Count
    foreach ($img in $images) {                                                 # ICONDIRENTRY
        $s = $img[0]; $data = $img[1]
        $dim = if ($s -ge 256) { 0 } else { $s }
        $w.Write([byte]$dim); $w.Write([byte]$dim); $w.Write([byte]0); $w.Write([byte]0)
        $w.Write([uint16]1); $w.Write([uint16]32)
        $w.Write([uint32]$data.Length); $w.Write([uint32]$offset)
        $offset += $data.Length
    }
    foreach ($img in $images) { $w.Write([byte[]]$img[1]) }
    $w.Flush()
    [System.IO.File]::WriteAllBytes($dst, $out.ToArray())
    $w.Dispose()
}

function Find-Iscc {
    if ($env:ISCC -and (Test-Path $env:ISCC)) { return $env:ISCC }
    $cmd = Get-Command ISCC.exe -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    $bases = @(${env:ProgramFiles(x86)}, $env:ProgramFiles, (Join-Path $env:LOCALAPPDATA 'Programs')) | Where-Object { $_ }
    foreach ($b in $bases) {
        foreach ($v in 'Inno Setup 6', 'Inno Setup 7') {
            $p = Join-Path $b "$v\ISCC.exe"
            if (Test-Path $p) { return $p }
        }
    }
    foreach ($key in 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Inno Setup 6_is1',
                     'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Inno Setup 6_is1',
                     'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Inno Setup 6_is1') {
        $item = Get-ItemProperty $key -ErrorAction SilentlyContinue
        if (-not $item -or -not $item.PSObject.Properties['InstallLocation']) { continue }
        $p = Join-Path $item.InstallLocation 'ISCC.exe'
        if (Test-Path $p) { return $p }
    }
    return $null
}

Push-Location $Root
try {
    # Version from Cargo.toml: binary, installer and update check always agree.
    $Version = (Select-String -Path (Join-Path $Root 'Cargo.toml') -Pattern '^version\s*=\s*"([^"]+)"' |
        Select-Object -First 1).Matches[0].Groups[1].Value
    if (-not $Version) { throw 'version not found in Cargo.toml' }

    Step 1 'Application icon (assets\crow.ico)'
    if ($RegenerateIcon -or -not (Test-Path $Ico)) {
        if (-not (Test-Path $Png)) { throw "missing $Png" }
        New-CrowIcon $Png $Ico
        Write-Host "      generated $Ico"
    } else {
        Write-Host "      present"
    }

    Step 2 "Release build ($Target, v$Version)"
    if ($SkipBuild) {
        Write-Host '      skipped (-SkipBuild)'
    } else {
        & cargo build --release --target $Target
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed ($LASTEXITCODE)" }
    }
    if (-not (Test-Path $Exe)) { throw "missing $Exe" }

    Step 3 'Binary size'
    # Symbols are already stripped by the release profile (`strip = true`, thin LTO); no packer
    # (UPX & co. trip antivirus heuristics and slow every start).
    $mb = '{0:N1}' -f ((Get-Item $Exe).Length / 1MB)
    Write-Host "      crow-browser.exe: $mb MB (stripped by the release profile)"
    $sign = Get-Command signtool.exe -ErrorAction SilentlyContinue
    if ($sign -and $env:CROW_SIGN_CERT) {
        & $sign.Source sign /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 /f $env:CROW_SIGN_CERT $Exe
        if ($LASTEXITCODE -ne 0) { throw 'signtool failed' }
    }

    Step 4 'Inno Setup compiler'
    $Iscc = Find-Iscc
    if (-not $Iscc) {
        throw 'ISCC.exe not found. Install Inno Setup 6 (winget install JRSoftware.InnoSetup) or set $env:ISCC.'
    }
    Write-Host "      $Iscc"

    Step 5 'Installer'
    New-Item -ItemType Directory -Force $Dist | Out-Null
        & $Iscc /Qp "/DAppVersion=$Version" $Iss
    if ($LASTEXITCODE -ne 0) { throw "ISCC failed ($LASTEXITCODE)" }
    $Setup = Join-Path $Dist "crow-browser-setup-v$Version.exe"
    if (-not (Test-Path $Setup)) { throw "installer not produced: $Setup" }
    if ($sign -and $env:CROW_SIGN_CERT) {
        & $sign.Source sign /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 /f $env:CROW_SIGN_CERT $Setup
        if ($LASTEXITCODE -ne 0) { throw 'signtool failed (installer)' }
    }

    Step 6 'Done'
    $hash = (Get-FileHash -Algorithm SHA256 $Setup).Hash.ToLower()
    Write-Host ''
    Write-Host "Installer: $Setup" -ForegroundColor Green
    Write-Host "SHA-256:   $hash"
} finally {
    Pop-Location
}
