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
# Only "access denied" counts as the folder's permissions saying no. A missing folder or any other
# error is a failure, so a setup mistake cannot look like a pass.
function Denied-Write([string] $Folder, [string] $Name) {
  if (-not (Test-Path -LiteralPath $Folder -PathType Container)) { return @($false, 'folder missing') }
  $target = Join-Path $Folder $Name
  try { [IO.File]::WriteAllBytes($target, [byte[]](1, 2, 3)); Remove-Item -LiteralPath $target -Force -ErrorAction SilentlyContinue; return @($false, 'write succeeded') }
  catch [UnauthorizedAccessException] { return @($true, '') }
  catch { return @($false, $_.Exception.GetType().Name) }
}
# A running program cannot be written to whatever its permissions say, so test a rename instead:
# Windows allows renaming a running program unless the permissions forbid it.
function Denied-Rename([string] $File) {
  if (-not (Test-Path -LiteralPath $File -PathType Leaf)) { return @($false, 'file missing') }
  $moved = "$File.renamed"
  try { [IO.File]::Move($File, $moved); return @($false, 'rename succeeded') }
  catch [UnauthorizedAccessException] { return @($true, '') }
  catch { return @($false, $_.Exception.GetType().Name) }
  finally { if (Test-Path -LiteralPath $moved) { [IO.File]::Move($moved, $File) } }
}
function Check([string] $name, [bool] $ok, [string] $detail = '') { $lines.Add(('{0} {1}{2}' -f ($(if ($ok) { 'PASS' } else { 'FAIL' })), $name, $(if ($detail) { "  [$detail]" } else { '' }))) }

$me = [Security.Principal.WindowsIdentity]::GetCurrent()
$admin = ([Security.Principal.WindowsPrincipal]$me).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
Check 'running as a normal (non-elevated) user' (-not $admin) $me.Name

foreach ($t in @(@('service folder', $Dir), @('staging folder', "$Dir\staging"))) {
  $r = Denied-Write $t[1] 'planted.txt'
  Check ("normal user can NOT write to the " + $t[0]) $r[0] $r[1]
}
$r = Denied-Rename "$Dir\artcraft-updater-service.exe"
Check 'normal user can NOT rename the service program' $r[0] $r[1]
$stop = (& "$env:SystemRoot\System32\sc.exe" stop ArtCraftUpdater 2>&1) -join ' '
Check 'normal user can NOT stop the service' ($stop -match 'Access is denied|FAILED 5') $stop.Substring(0, [Math]::Min(80, $stop.Length))

$env:E2E_FROM = $From; $env:E2E_TO = $To
$run = & $TestExe --ignored --nocapture --test-threads=1 an_all_users_app 2>&1 | Out-String
$code = $LASTEXITCODE
foreach ($l in ($run -split "`r?`n")) { if ($l.Trim()) { $lines.Add("  | $l") } }
Check "the launcher's install path updated the app through the service (test exit=$code)" ($code -eq 0)
Set-Content -Path $Out -Value $lines
