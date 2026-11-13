<#
.SYNOPSIS
  Build, sign and package VectorCraft (or StitchCraft, with -App stitchcraft) for Windows.

.DESCRIPTION
  Produces, in $env:DIST (default: dist/release):
    vectorcraft-<version>-windows-<arch>.msi            per-machine installer (WiX v5)
    vectorcraft-<version>-windows-<arch>-portable.zip   vectorcraft.exe + vectorcraft-cli.exe + portable.txt

  With -App stitchcraft the same two files for StitchCraft, from the same WiX sources with the
  app's names, ids and icon filled in (ConvertTo-StitchCraft) and its own upgrade code:
    stitchcraft-<version>-windows-<arch>.msi
    stitchcraft-<version>-windows-<arch>.zip            stitchcraft.exe + stitchcraft-cli.exe, no portable.txt
  (portable mode, which settings in a folder beside the exe need, is VectorCraft's alone:
  apps/vectorcraft/src/profile.rs).

  The binaries link the C runtime statically (+crt-static), so neither the MSI nor the portable
  zip needs the Visual C++ redistributable. Signing is delegated to sign.ps1 (skipped with a
  warning when no signing secrets are set).

  Needs: Rust (MSVC toolchain + the target), the Windows SDK (rc.exe, signtool.exe),
  and WiX v5: dotnet tool install --global wix --version 5.0.2
             wix extension add -g WixToolset.UI.wixext/5.0.2 WixToolset.Util.wixext/5.0.2

.EXAMPLE
  pwsh packaging/windows/package.ps1 -Arch x64
  pwsh packaging/windows/package.ps1 -App stitchcraft -Arch x64
  pwsh packaging/windows/package.ps1 -Arch x86 -SkipBuild
  pwsh packaging/windows/package.ps1 -Arch arm64     # cross-compiled; needs the MSVC ARM64 build tools
#>
param(
  [ValidateSet('vectorcraft', 'stitchcraft')] [string] $App = 'vectorcraft',
  [ValidateSet('x64', 'x86', 'arm64')] [string] $Arch = 'x64',
  [switch] $SkipBuild
)
$ErrorActionPreference = 'Stop'
$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path

function Invoke-Native([string] $What, [scriptblock] $Block) {
  Write-Output "==> $What"
  & $Block
  if ($LASTEXITCODE -ne 0) { throw "$What failed with exit code $LASTEXITCODE" }
}

# What differs between the apps: the name people see, and the CLI's name in the package (the build
# is always vectorcraft-cli.exe; StitchCraft's is installed as stitchcraft-cli.exe).
$Name = if ($App -eq 'stitchcraft') { 'StitchCraft' } else { 'VectorCraft' }
$Cli = "$App-cli"

# The version lives in one place: [workspace.package] version in the root Cargo.toml.
$Version = $env:VECTORCRAFT_VERSION
if (-not $Version) {
  $inPkg = $false
  foreach ($line in Get-Content (Join-Path $Root 'Cargo.toml')) {
    if ($line -match '^\s*\[') { $inPkg = ($line.Trim() -eq '[workspace.package]'); continue }
    if ($inPkg -and $line -match '^\s*version\s*=\s*"([^"]+)"') { $Version = $Matches[1]; break }
  }
}
if (-not $Version) { throw 'could not read [workspace.package] version from Cargo.toml' }
# MSI ProductVersion is numeric (major.minor.build); pre-release tags are dropped there.
$MsiVersion = ($Version -split '-')[0]

$Target = switch ($Arch) { 'x64' { 'x86_64-pc-windows-msvc' } 'x86' { 'i686-pc-windows-msvc' } 'arm64' { 'aarch64-pc-windows-msvc' } }
$Dist = if ($env:DIST) { $env:DIST } else { Join-Path $Root 'dist\release' }
$TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Root 'target' }
New-Item -ItemType Directory -Force -Path $Dist | Out-Null

if (-not $env:VECTORCRAFT_BUILD_SHA) { $env:VECTORCRAFT_BUILD_SHA = (git -C $Root rev-parse HEAD 2>$null) }
if (-not $env:VECTORCRAFT_BUILD_DATE) { $env:VECTORCRAFT_BUILD_DATE = (Get-Date).ToUniversalTime().ToString('yyyy-MM-dd') }

Write-Output "$Name $Version for Windows $Arch ($Target)"

if (-not $SkipBuild) {
  # Static CRT: no VC++ redistributable needed. Scoped to the target so host build scripts and
  # proc-macros are unaffected.
  $flagVar = 'CARGO_TARGET_' + ($Target.ToUpper() -replace '-', '_') + '_RUSTFLAGS'
  [Environment]::SetEnvironmentVariable($flagVar, '-C target-feature=+crt-static')
  # Fail the build (rather than warn) if the icon/VERSIONINFO can't be embedded.
  $env:VECTORCRAFT_REQUIRE_WINRES = '1'
  Invoke-Native "cargo build ($Target)" { cargo build --release --locked -p $App -p vectorcraft-cli --target $Target }
}

$Bin = Join-Path $TargetDir "$Target\release"

# Check both binaries' PE headers before packaging. Machine (COFF header) must match -Arch, so an
# x64 build can never ship labelled arm64. Subsystem (optional header): 2 = Windows GUI, 3 = console.
# The app must be GUI (no console window opens with it); the CLI must stay console so its output
# reaches the terminal.
function Get-PeHeader([string] $Path) {
  $bytes = [System.IO.File]::ReadAllBytes($Path)
  $pe = [BitConverter]::ToInt32($bytes, 0x3C)
  return @{ Machine = [BitConverter]::ToUInt16($bytes, $pe + 4); Subsystem = [BitConverter]::ToUInt16($bytes, $pe + 0x5C) }
}
$Machine = switch ($Arch) { 'x64' { 0x8664 } 'x86' { 0x14C } 'arm64' { 0xAA64 } }
foreach ($check in @(@("$App.exe", 2), @('vectorcraft-cli.exe', 3))) {
  $h = Get-PeHeader (Join-Path $Bin $check[0])
  if ($h.Machine -ne $Machine) { throw "$($check[0]) is for machine 0x$('{0:X}' -f $h.Machine), expected 0x$('{0:X}' -f $Machine) ($Arch)" }
  if ($h.Subsystem -ne $check[1]) { throw "$($check[0]) has PE subsystem $($h.Subsystem), expected $($check[1])" }
  Write-Output "ok $($check[0]): $Arch, PE subsystem $($h.Subsystem)"
}
$Stage = Join-Path $TargetDir "windows-package\$Arch"
Remove-Item -Recurse -Force $Stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $Stage | Out-Null
Copy-Item (Join-Path $Bin "$App.exe") $Stage
Copy-Item (Join-Path $Bin 'vectorcraft-cli.exe') (Join-Path $Stage "$Cli.exe")

& (Join-Path $PSScriptRoot 'sign.ps1') (Join-Path $Stage "$App.exe") (Join-Path $Stage "$Cli.exe")

# Documentation and dependency licences shared by the MSI and portable ZIP.
$Docs = Join-Path $Stage 'docs'
New-Item -ItemType Directory -Force -Path $Docs | Out-Null
foreach ($f in 'README.md', 'LICENSE', 'LICENSE-MIT', 'LICENSE-APACHE', 'NOTICE', 'ASSETS.md') {
  $p = Join-Path $Root $f
  if (Test-Path $p) { Copy-Item $p $Docs }
}
# Built with craft-fonts (CRAFT_FONTS_DIR, release builds): the embedded fonts' licences.
if ($env:CRAFT_FONTS_DIR) {
  foreach ($ofl in Get-ChildItem -Path (Join-Path $env:CRAFT_FONTS_DIR 'fonts\*\OFL.txt') -ErrorAction SilentlyContinue) {
    Copy-Item $ofl.FullName (Join-Path $Docs "OFL-$($ofl.Directory.Name).txt")
  }
}
if (Test-Path (Join-Path $Root 'licenses')) {
  Copy-Item -Recurse (Join-Path $Root 'licenses') $Docs
}

# ---- MSI ---------------------------------------------------------------------------------------
# The setup wizard's art, drawn here from the app icon so the MSI shows the app rather than
# WiX's stock bitmaps: a 493x58 banner across the top of the inner pages (the page title is drawn
# over its left side, so that stays white) and a 493x312 backdrop for the Welcome and Finish pages
# (their text sits on the right; the left 164 px are ours). Both must be 24-bit BMPs.
function New-InstallerArt([string] $IconPng, [string] $BannerOut, [string] $DialogOut) {
  Add-Type -AssemblyName System.Drawing
  $paper = [System.Drawing.ColorTranslator]::FromHtml('#efe9dc')
  $accent = [System.Drawing.ColorTranslator]::FromHtml('#e8573f')
  $icon = [System.Drawing.Image]::FromFile($IconPng)
  try {
    foreach ($spec in @(@($BannerOut, 493, 58), @($DialogOut, 493, 312))) {
      $bmp = New-Object System.Drawing.Bitmap $spec[1], $spec[2], ([System.Drawing.Imaging.PixelFormat]::Format24bppRgb)
      $g = [System.Drawing.Graphics]::FromImage($bmp)
      try {
        $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
        $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
        $g.Clear([System.Drawing.Color]::White)
        if ($spec[2] -eq 58) {
          $g.DrawImage($icon, 493 - 46 - 8, 6, 46, 46)
        } else {
          $panel = New-Object System.Drawing.SolidBrush $paper
          $stripe = New-Object System.Drawing.SolidBrush $accent
          $g.FillRectangle($panel, 0, 0, 164, 312)
          $g.FillRectangle($stripe, 160, 0, 4, 312)
          $g.DrawImage($icon, 18, 64, 124, 124)
          $panel.Dispose(); $stripe.Dispose()
        }
      } finally { $g.Dispose() }
      $bmp.Save($spec[0], [System.Drawing.Imaging.ImageFormat]::Bmp)
      $bmp.Dispose()
    }
  } finally { $icon.Dispose() }
}
$Banner = Join-Path $Stage 'installer-banner.bmp'
$Dialog = Join-Path $Stage 'installer-dialog.bmp'
New-InstallerArt (Join-Path $Root "assets\app-icon\hicolor\256x256\apps\ai.storyteller.$App.png") $Banner $Dialog

# StitchCraft's installer is VectorCraft's, written again with StitchCraft's own words: the sources
# (the WiX file, its include, the wizard's strings and the install policy the check below reads) are
# copied into the stage with the names, ids and registry keys changed, so the two cannot drift apart.
# What stays VectorCraft's is the page ARP links to (StitchCraft has none yet). Every source must
# hold what is replaced, or the build stops.
$VectorCraftUpgradeCode = '601C6437-08AE-4645-ADCC-27F036C2EA2F'
$StitchCraftIds = @{
  $VectorCraftUpgradeCode                = '777F2F5E-782C-4B5B-B05D-E0ACB06CFEB1'  # the product
  '0E8C6C5E-DE4A-4E1D-9DB0-44E8E38C2103' = '73E34536-F54B-4492-8B26-ABF426AB6D98'  # the app's component
  '498F2D4C-E655-47F6-B406-2D4C449B8D26' = '9E2CC842-1B2C-41F9-9CBF-60FE282B7FD0'  # the desktop shortcut's
}
function ConvertTo-StitchCraft([string] $Text, [string] $What) {
  foreach ($id in $StitchCraftIds.Keys) {
    # The policy file names only the product; the WiX file names all three.
    if ($Text.Contains($id)) { $Text = $Text.Replace($id, $StitchCraftIds[$id]) }
  }
  $page = 'https://getartcraft.com/apps/vectorcraft'
  # The CLI keeps the build's name until here, and the page its own, through the renames.
  $Text = $Text.Replace('vectorcraft-cli', '@@CLI@@').Replace($page, '@@PAGE@@')
  $Text = $Text.Replace('Vectorcraft', 'Stitchcraft').Replace('VectorCraft', 'StitchCraft').Replace('vectorcraft', 'stitchcraft')
  if ($Text -match '(?i)vectorcraft') { throw "$What still names VectorCraft after the rename" }
  return $Text.Replace('@@CLI@@', 'stitchcraft-cli').Replace('@@PAGE@@', $page)
}
$Wix = $PSScriptRoot
$PolicyPath = Join-Path $PSScriptRoot 'install-policy.json'
$IconIco = Join-Path $Root "assets\app-icon\$App.ico"
if ($App -eq 'stitchcraft') {
  $Wix = Join-Path $Stage 'wix'
  New-Item -ItemType Directory -Force -Path $Wix | Out-Null
  foreach ($pair in @(@('vectorcraft.wxs', 'stitchcraft.wxs'), @('install-policy.wxi', 'install-policy.wxi'),
                      @('vectorcraft.en-us.wxl', 'stitchcraft.en-us.wxl'), @('install-policy.json', 'install-policy.json'))) {
    $text = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot $pair[0])
    if ($text -notmatch 'vectorcraft') { throw "$($pair[0]) no longer names VectorCraft: package.ps1 cannot make StitchCraft's from it" }
    Set-Content -LiteralPath (Join-Path $Wix $pair[1]) -Value (ConvertTo-StitchCraft $text $pair[0]) -NoNewline -Encoding utf8
  }
  $PolicyPath = Join-Path $Wix 'install-policy.json'
}

$Msi = Join-Path $Dist "$App-$Version-windows-$Arch.msi"
Invoke-Native 'wix build' {
  wix build (Join-Path $Wix "$App.wxs") -arch $Arch `
    -ext WixToolset.UI.wixext -ext WixToolset.Util.wixext `
    -culture en-US -loc (Join-Path $Wix "$App.en-us.wxl") `
    -d "Version=$MsiVersion" -d "DisplayVersion=$Version" -d "BinDir=$Stage" -d "IconPath=$IconIco" `
    -d "DocsDir=$Docs" -d "BannerBmp=$Banner" -d "DialogBmp=$Dialog" `
    -o $Msi
}
# Check the compiled scope, path and registration contract before signing.
& (Join-Path $PSScriptRoot 'check-install-policy.ps1') -MsiPath $Msi -PolicyPath $PolicyPath
# wix writes its debug symbols (.wixpdb) next to the MSI; keep them out of the release assets.
Remove-Item -Force -ErrorAction SilentlyContinue ([IO.Path]::ChangeExtension($Msi, '.wixpdb'))
& (Join-Path $PSScriptRoot 'sign.ps1') $Msi

# ---- zip ---------------------------------------------------------------------------------------
# VectorCraft's is portable; StitchCraft's holds the same files without the marker (see above).
$ZipName = if ($App -eq 'vectorcraft') { "vectorcraft-$Version-windows-$Arch-portable" } else { "$App-$Version-windows-$Arch" }
$Portable = Join-Path $TargetDir "windows-package\$ZipName"
Remove-Item -Recurse -Force $Portable -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $Portable | Out-Null
Copy-Item (Join-Path $Stage '*.exe') $Portable
Copy-Item -Recurse (Join-Path $Docs '*') $Portable
# portable.txt beside vectorcraft.exe switches on portable mode: settings, libraries, recovery
# files and logs go to VectorCraftData\ next to the exe instead of %APPDATA% (#1366; see profile.rs).
if ($App -eq 'vectorcraft') { Copy-Item (Join-Path $PSScriptRoot 'portable.txt') $Portable }
$Zip = Join-Path $Dist "$ZipName.zip"
Remove-Item -Force $Zip -ErrorAction SilentlyContinue
Compress-Archive -Path $Portable -DestinationPath $Zip

# Smoke-test the CLI when this machine can run it. An ARM64 build made on an x64 runner can't run
# here; .github/workflows/windows-arm64.yml installs and runs it on ARM64 instead.
$HostArch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString().ToLowerInvariant()
if ($Arch -ne 'arm64' -or $HostArch -eq 'arm64') {
  Invoke-Native "$Cli --version" { & (Join-Path $Stage "$Cli.exe") --version }
} else {
  Write-Output "skipping $Cli --version: an $Arch build doesn't run on this $HostArch machine"
}
Get-Item $Msi, $Zip | Format-Table Name, Length
