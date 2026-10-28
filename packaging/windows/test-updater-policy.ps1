<#
.SYNOPSIS
  Build non-executable ArtCraft Updater installer fixtures and check their compiled policy.
  Never installs anything.
#>
param(
  [Parameter(Mandatory = $true)] [string] $OutputDirectory,
  [ValidateSet('x64','x86','arm64')] [string[]] $Architectures = @('x64','x86','arm64')
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$policy = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot 'updater-install-policy.json') | ConvertFrom-Json
[void](New-Item -ItemType Directory -Force -Path $OutputDirectory)
$output = (Resolve-Path -LiteralPath $OutputDirectory).Path
foreach ($arch in $Architectures) {
  $stage = Join-Path $output $arch
  [void](New-Item -ItemType Directory -Force -Path $stage)
  foreach ($name in $policy.payloads) {
    Set-Content -LiteralPath (Join-Path $stage $name) -Encoding Ascii -Value 'NON-EXECUTABLE MSI METADATA TEST FIXTURE'
  }
  $sources = @($policy.sources | ForEach-Object { Join-Path $PSScriptRoot $_ })
  $msi = Join-Path $stage "$($policy.id)-NOT-INSTALLABLE.msi"
  $extensions = @($policy.extensions | ForEach-Object { '-ext'; "WixToolset.$_.wixext" })
  & wix build @sources -arch $arch @extensions -culture en-US -d 'Version=0.0.1' -d "BinDir=$stage" -d "IconPath=$(Join-Path $root $policy.icon)" -o $msi
  if ($LASTEXITCODE -ne 0) { throw "WiX build failed for $arch (exit $LASTEXITCODE)." }
  $shell = (Get-Process -Id $PID).Path
  & $shell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'check-updater-policy.ps1') -MsiPath $msi
  if ($LASTEXITCODE -ne 0) { throw "Compiled updater policy failed for $arch (exit $LASTEXITCODE)." }
  Write-Output "PASS $arch metadata-only package. This is not an executable installer."
}
