# Runs as a temporary STANDARD Windows account (started by e2e.ps1). Writes PASS/FAIL lines to -Out.
param([Parameter(Mandatory = $true)][string] $Out, [Parameter(Mandatory = $true)][string] $Client)
$ErrorActionPreference = 'Continue'
$Dir = 'C:\Program Files\ArtCraftUpdaterE2E'
$lines = New-Object System.Collections.Generic.List[string]
function Check([string] $name, [bool] $ok, [string] $detail = '') {
  $lines.Add(('{0} {1}{2}' -f ($(if ($ok) { 'PASS' } else { 'FAIL' })), $name, $(if ($detail) { "  [$detail]" } else { '' })))
}

$me = [Security.Principal.WindowsIdentity]::GetCurrent()
$admin = ([Security.Principal.WindowsPrincipal]$me).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
Check 'second account is a standard user (not an administrator)' (-not $admin) $me.Name
Check 'second account is the temporary test account' ($me.Name -match '\ArtCraftE2EUser$') $me.Name

# The pipe, from a different account.
$run = & $Client --ignored --nocapture --test-threads=1 2>&1 | Out-String
$replies = @{}
foreach ($l in ($run -split "`r?`n")) {
  $i = $l.IndexOf('REPLY ')
  if ($i -ge 0) { try { $o = $l.Substring($i + 6) | ConvertFrom-Json; $replies[$o.label] = ($o.reply | ConvertTo-Json -Compress) } catch {} }
}
Check 'pipe is reachable by a standard user' ($replies.Count -ge 7) ("replies=" + $replies.Count + $(if ($replies.Count -lt 7) { " out=" + $run.Substring(0, [Math]::Min(300, $run.Length)) }))
Check 'unknown app refused in plain words' ($replies['unknown-app'] -match 'not part of the Craft App Suite') $replies['unknown-app']
Check 'garbage refused in plain words' ($replies['garbage'] -match 'not understood') $replies['garbage']
Check 'oversize refused' ($replies['oversize'] -match 'too large') $replies['oversize']
Check 'app not installed per-machine is not installed for all users' ($replies['not-installed'] -match 'not installed for all users') $replies['not-installed']

foreach ($t in @(@('service folder', "$Dir\planted.msi"), @('staging folder', "$Dir\staging\planted.msi"), @('service exe', "$Dir\artcraft-updater-service.exe"))) {
  $wrote = $false
  try { [IO.File]::WriteAllBytes($t[1], [byte[]](1, 2, 3)); $wrote = $true; Remove-Item $t[1] -Force -ErrorAction SilentlyContinue } catch {}
  Check ("standard user can NOT write to the " + $t[0]) (-not $wrote)
}
$stop = (& "$env:SystemRoot\System32\sc.exe" stop ArtCraftUpdaterE2E 2>&1) -join ' '
Check 'standard user can NOT stop the service' ($stop -match 'Access is denied|FAILED 5') $stop.Substring(0, [Math]::Min(100, $stop.Length))

Set-Content -Path $Out -Value $lines
