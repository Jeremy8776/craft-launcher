<#
.SYNOPSIS
  Build, sign and package the ArtCraft Updater (the all-users update service) for Windows.

.DESCRIPTION
  Produces, in $env:DIST (default: dist/release):
    artcraft-updater-<version>-windows-<arch>.msi    per-machine installer for the service (WiX v5)

  Separate from package.ps1 on purpose: ArtCraft Launcher's own installer is per-user and is what
  the launcher updates itself through. This one needs Windows' permission prompt once, installs
  one service into Program Files and nothing else.

  Needs: Rust (MSVC toolchain + the target) and WiX v5 (dotnet tool install --global wix --version 5.0.2).
  No WiX extensions are used. Signing is delegated to sign.ps1 (skipped with a warning when no
  signing secrets are set).

.EXAMPLE
  pwsh packaging/windows/package-updater.ps1 -Arch x64
  pwsh packaging/windows/package-updater.ps1 -Arch x64 -SkipBuild
#>
param(
  [ValidateSet('x64', 'x86', 'arm64')] [string] $Arch = 'x64',
  [switch] $SkipBuild
)
$ErrorActionPreference = 'Stop'
$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path

$Pkg = 'artcraft-launcher-updater-service'   # cargo package
$Exe = 'artcraft-updater-service.exe'        # its binary
$AppName = 'ArtCraft Updater'
$Id = 'artcraft-updater'

function Invoke-Native([string] $What, [scriptblock] $Block) {
  Write-Output "==> $What"
  & $Block
  if ($LASTEXITCODE -ne 0) { throw "$What failed with exit code $LASTEXITCODE" }
}

# The version lives in one place: [workspace.package] version in the root Cargo.toml.
$Version = $env:ARTCRAFT_LAUNCHER_VERSION
if (-not $Version) {
  $inPkg = $false
  foreach ($line in Get-Content (Join-Path $Root 'Cargo.toml')) {
    if ($line -match '^\s*\[') { $inPkg = ($line.Trim() -eq '[workspace.package]'); continue }
    if ($inPkg -and $line -match '^\s*version\s*=\s*"([^"]+)"') { $Version = $Matches[1]; break }
  }
}
if (-not $Version) { throw 'could not read [workspace.package] version from Cargo.toml' }
# MSI ProductVersion is numeric; pre-release tags are dropped there.
$MsiVersion = ($Version -split '-')[0]

$Target = switch ($Arch) { 'x64' { 'x86_64-pc-windows-msvc' } 'x86' { 'i686-pc-windows-msvc' } 'arm64' { 'aarch64-pc-windows-msvc' } }
$Dist = if ($env:DIST) { $env:DIST } else { Join-Path $Root 'dist\release' }
$TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Root 'target' }
New-Item -ItemType Directory -Force -Path $Dist | Out-Null
Write-Output "$AppName $Version for Windows $Arch ($Target)"

if (-not $SkipBuild) {
  # Static CRT: no VC++ redistributable needed.
  $flagVar = 'CARGO_TARGET_' + ($Target.ToUpper() -replace '-', '_') + '_RUSTFLAGS'
  [Environment]::SetEnvironmentVariable($flagVar, '-C target-feature=+crt-static')
  Invoke-Native "cargo build ($Target)" { cargo build --release --locked -p $Pkg --target $Target }
}

$Bin = Join-Path $TargetDir "$Target\release"
$Stage = Join-Path $TargetDir "windows-package\$Id-$Arch"
Remove-Item -Recurse -Force $Stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $Stage | Out-Null
Copy-Item (Join-Path $Bin $Exe) $Stage

# Same PE checks as the launcher: the machine must match -Arch; a service is a console-subsystem
# program (subsystem 3) that the service manager starts.
function Get-PeHeader([string] $Path) {
  $bytes = [System.IO.File]::ReadAllBytes($Path)
  $pe = [BitConverter]::ToInt32($bytes, 0x3C)
  return @{ Machine = [BitConverter]::ToUInt16($bytes, $pe + 4); Subsystem = [BitConverter]::ToUInt16($bytes, $pe + 0x5C) }
}
$Machine = switch ($Arch) { 'x64' { 0x8664 } 'x86' { 0x14C } 'arm64' { 0xAA64 } }
$h = Get-PeHeader (Join-Path $Stage $Exe)
if ($h.Machine -ne $Machine) { throw "$Exe is for machine 0x$('{0:X}' -f $h.Machine), expected 0x$('{0:X}' -f $Machine) ($Arch)" }
Write-Output "ok ${Exe}: $Arch, PE subsystem $($h.Subsystem)"

& (Join-Path $PSScriptRoot 'sign.ps1') (Join-Path $Stage $Exe)

$Msi = Join-Path $Dist "$Id-$Version-windows-$Arch.msi"
Invoke-Native 'wix build' {
  wix build (Join-Path $PSScriptRoot "$Id.wxs") -arch $Arch `
    -d "Version=$MsiVersion" -d "BinDir=$Stage" -d "IconPath=$(Join-Path $Root 'assets\app-icon\artcraft-launcher.ico')" `
    -o $Msi
}
# Nothing is installed here: this only reads the package's tables.
& (Join-Path $PSScriptRoot 'check-updater-policy.ps1') -MsiPath $Msi
# wix writes its debug symbols (.wixpdb) next to the MSI; keep them out of the release assets.
Remove-Item -Force -ErrorAction SilentlyContinue ([IO.Path]::ChangeExtension($Msi, '.wixpdb'))
& (Join-Path $PSScriptRoot 'sign.ps1') $Msi

Get-Item $Msi | Format-Table Name, Length
