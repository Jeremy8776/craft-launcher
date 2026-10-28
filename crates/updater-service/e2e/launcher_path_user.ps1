# Normal-user half of the launcher-path run. Must NOT be elevated. Writes lp-user-results.txt.
param(
  [Parameter(Mandatory = $true)][string] $Root,
  [Parameter(Mandatory = $true)][string] $TestExe,
  [Parameter(Mandatory = $true)][string] $From,
  [Parameter(Mandatory = $true)][string] $To
)
$ErrorActionPreference = 'Continue'
$Out = Join-Path $Root 'lp-user-results.txt'
$Dir = 'C:\Program Files\ArtCraft\Updater'
$lines = New-Object System.Collections.Generic.List[string]
function Check([string] $name, [bool] $ok, [string] $detail = '') { $lines.Add(('{0} {1}{2}' -f ($(if ($ok) { 'PASS' } else { 'FAIL' })), $name, $(if ($detail) { "  [$detail]" } else { '' }))) }

$me = [Security.Principal.WindowsIdentity]::GetCurrent()
$admin = ([Security.Principal.WindowsPrincipal]$me).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
Check 'running as a normal (non-elevated) user' (-not $admin) $me.Name

foreach ($t in @(@('service folder', "$Dir\planted.txt"), @('staging folder', "$Dir\staging\planted.txt"), @('service program', "$Dir\artcraft-updater-service.exe"))) {
  $wrote = $false
  try { [IO.File]::WriteAllBytes($t[1], [byte[]](1, 2, 3)); $wrote = $true; Remove-Item $t[1] -Force -ErrorAction SilentlyContinue } catch {}
  Check ("normal user can NOT write to the " + $t[0]) (-not $wrote)
}
$stop = (& "$env:SystemRoot\System32\sc.exe" stop ArtCraftUpdater 2>&1) -join ' '
Check 'normal user can NOT stop the service' ($stop -match 'Access is denied|FAILED 5') $stop.Substring(0, [Math]::Min(80, $stop.Length))

$env:E2E_FROM = $From; $env:E2E_TO = $To
$run = & $TestExe --ignored --nocapture --test-threads=1 an_all_users_app 2>&1 | Out-String
$code = $LASTEXITCODE
foreach ($l in ($run -split "`r?`n")) { if ($l.Trim()) { $lines.Add("  | $l") } }
Check "the launcher's install path updated the app through the service (test exit=$code)" ($code -eq 0)
Set-Content -Path $Out -Value $lines
