# Elevated half of the updater service end-to-end test. Started once, elevated, by the agent
# (the user answers the single UAC prompt). Everything is cleaned up in `finally`.
#
#   Part 1: the engine's real msiexec command line updates a renamed per-machine test product
#           (IptzUpdaterE2E 1.2.2 -> 1.2.3), refuses a downgrade, and is removed again.
#   Part 2: the real service binary is registered as ArtCraftUpdaterE2E (LocalSystem) in a
#           test folder under Program Files, started, and held up while a NON-elevated client
#           talks to it. Then it is stopped and deleted.
param(
  [Parameter(Mandatory = $true)][string] $Root,
  [Parameter(Mandatory = $true)][string] $ServiceExe,
  [Parameter(Mandatory = $true)][string] $InstallTest,
  [switch] $SkipInstallTest
)
$ErrorActionPreference = 'Continue'
$Log = Join-Path $Root 'elevated-results.txt'
$Dir = 'C:\Program Files\ArtCraftUpdaterE2E'
$Svc = 'ArtCraftUpdaterE2E'
function Log([string] $m) { Add-Content -Path $Log -Value ('{0} {1}' -f (Get-Date -Format 'HH:mm:ss'), $m) }
function WaitFor([scriptblock] $cond, [int] $seconds) {
  $until = (Get-Date).AddSeconds($seconds)
  while ((Get-Date) -lt $until) { if (& $cond) { return $true }; Start-Sleep -Milliseconds 300 }
  return $false
}
Remove-Item (Join-Path $Root 'service-ready'), (Join-Path $Root 'client-done'), (Join-Path $Root 'elevated-finished') -Force -ErrorAction SilentlyContinue
Set-Content -Path $Log -Value ''
try {
  $elevated = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
  Log "elevated=$elevated"
  if (-not $elevated) { throw 'not elevated' }

  # ---- Part 1 ----
  Log 'PART 1: per-machine update with the real command line'
  if ($SkipInstallTest) { Log 'skipped (already passed)' } else {
    $env:E2E_MSI_DIR = Join-Path $Root 'msi'
    $out = & $InstallTest --ignored --nocapture --test-threads=1 2>&1
    $code = $LASTEXITCODE
    $out | ForEach-Object { Log "  | $_" }
    Log "PART1 exit=$code"
  }

  # ---- Part 2 ----
  Log 'PART 2: the real service under LocalSystem'
  New-Item -ItemType Directory -Path $Dir -Force | Out-Null
  Copy-Item -LiteralPath $ServiceExe -Destination (Join-Path $Dir 'artcraft-updater-service.exe') -Force
  $bin = '"' + $Dir + '\artcraft-updater-service.exe" run'
  $created = New-Service -Name $Svc -BinaryPathName $bin -DisplayName 'ArtCraft Updater (test)' -StartupType Manual
  Log "New-Service: $($created.Name) $($created.Status)"
  $started = & sc.exe start $Svc 2>&1
  Log "sc start: $($started -join ' ')"
  $running = WaitFor { (Get-Service $Svc -ErrorAction SilentlyContinue).Status -eq 'Running' } 30
  Log "running=$running"
  $info = Get-CimInstance Win32_Service -Filter "Name='$Svc'"
  Log "service account=$($info.StartName) pid=$($info.ProcessId)"
  if ($info.ProcessId) {
    $owner = Invoke-CimMethod -InputObject (Get-CimInstance Win32_Process -Filter "ProcessId=$($info.ProcessId)") -MethodName GetOwner
    Log "process owner=$($owner.Domain)\$($owner.User)"
  }
  Log ("staging exists=" + (Test-Path (Join-Path $Dir 'staging')))
  Log ("staging owner/acl: " + ((Get-Acl (Join-Path $Dir 'staging')).Sddl))
  New-Item -ItemType File -Path (Join-Path $Root 'service-ready') -Force | Out-Null
  $answered = WaitFor { Test-Path (Join-Path $Root 'client-done') } 600
  Log "client finished=$answered"
}
catch { Log "ERROR: $_" }
finally {
  Log 'cleanup'
  & sc.exe stop $Svc 2>&1 | Out-Null
  [void](WaitFor { (Get-Service $Svc -ErrorAction SilentlyContinue).Status -ne 'Running' } 30)
  Log ("stopped, state=" + (Get-Service $Svc -ErrorAction SilentlyContinue).Status)
  $audit = Join-Path $Dir 'updater.log'
  if (Test-Path $audit) { Copy-Item $audit (Join-Path $Root 'service-audit.log') -Force }
  Log ("sc delete: " + ((& sc.exe delete $Svc 2>&1) -join ' '))
  Start-Sleep -Seconds 1
  Remove-Item -LiteralPath $Dir -Recurse -Force -ErrorAction SilentlyContinue
  # Belt and braces for part 1: remove the test product if the test did not.
  foreach ($v in 'v123', 'v122') {
    $msi = Get-ChildItem (Join-Path (Join-Path $Root 'msi') $v) -Filter *.msi -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($msi) { Start-Process msiexec.exe -ArgumentList '/x', "`"$($msi.FullName)`"", '/qn', '/norestart' -Wait }
  }
  Remove-Item -LiteralPath 'C:\Program Files\IptzUpdaterE2E' -Recurse -Force -ErrorAction SilentlyContinue
  Log ("left behind: service=" + [bool](Get-Service $Svc -ErrorAction SilentlyContinue) + " serviceDir=" + (Test-Path $Dir) + " testProductDir=" + (Test-Path 'C:\Program Files\IptzUpdaterE2E'))
  Log 'finished'
  New-Item -ItemType File -Path (Join-Path $Root 'elevated-finished') -Force | Out-Null
}
