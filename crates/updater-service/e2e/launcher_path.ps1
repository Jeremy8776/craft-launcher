# Elevated half of the launcher-path run (started once, elevated; the user answers the one UAC prompt).
#
#   1. Installs the REAL ArtCraft Updater MSI (the one package-updater.ps1 built) and checks what it set up.
#   2. Installs it again over itself, while the service runs (the upgrade path).
#   3. Installs the real signed CADCraft <From> per-machine.
#   4. Waits while a NORMAL (not elevated) user runs the launcher's install path against it.
#   5. Stops the service and runs the "no service" check.
#   6. Uninstalls everything and checks nothing is left behind.
# Everything is undone in `finally`, whatever happens.
param(
  [Parameter(Mandatory = $true)][string] $Root,
  [Parameter(Mandatory = $true)][string] $UpdaterMsi,
  [Parameter(Mandatory = $true)][string] $MsiFrom,
  [Parameter(Mandatory = $true)][string] $MsiTo,
  [Parameter(Mandatory = $true)][string] $TestExe
)
$ErrorActionPreference = 'Continue'
$Log = Join-Path $Root 'lp-elevated-results.txt'
$Dir = 'C:\Program Files\ArtCraft\Updater'
$Svc = 'ArtCraftUpdater'
$PfApp = 'C:\Program Files\CADCraft'
function Log([string] $m) { Add-Content -Path $Log -Value ('{0} {1}' -f (Get-Date -Format 'HH:mm:ss'), $m) }
function WaitFor([scriptblock] $cond, [int] $seconds) {
  $until = (Get-Date).AddSeconds($seconds)
  while ((Get-Date) -lt $until) { if (& $cond) { return $true }; Start-Sleep -Milliseconds 300 }
  return $false
}
function Msi([string[]] $arguments, [string] $logName) {
  $p = Start-Process msiexec.exe -ArgumentList ($arguments + @('/l*v', "`"$Root\$logName`"")) -Wait -PassThru
  return $p.ExitCode
}
function Service() { Get-CimInstance Win32_Service -Filter "Name='$Svc'" -ErrorAction SilentlyContinue }
Remove-Item (Join-Path $Root 'lp-ready'), (Join-Path $Root 'lp-client-done'), (Join-Path $Root 'lp-elevated-finished') -Force -ErrorAction SilentlyContinue
Set-Content -Path $Log -Value ''
try {
  $elevated = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
  Log "elevated=$elevated"
  if (-not $elevated) { throw 'not elevated' }
  if (Service) { throw 'an ArtCraftUpdater service already exists; not touching it' }
  if (Test-Path $PfApp) { throw 'C:\Program Files\CADCraft already exists; not touching it' }
  if (Test-Path "$env:LOCALAPPDATA\Programs\ArtCraft\CADCraft\cadcraft.exe") { throw 'a per-user CADCraft is still installed' }

  Log 'STEP 1: install the real updater MSI'
  $code = Msi @('/i', "`"$UpdaterMsi`"", '/qn', '/norestart') 'lp-updater-install.log'
  Log "msiexec exit=$code"
  $s = Service
  Log ("service: state=$($s.State) startMode=$($s.StartMode) account=$($s.StartName) path=$($s.PathName)")
  Log ("service pid=$($s.ProcessId) exe present=" + (Test-Path "$Dir\artcraft-updater-service.exe") + " pipe present=" + (Test-Path '\\.\pipe\artcraft-updater'))
  $reg = Get-ItemProperty 'HKLM:\SOFTWARE\Learning Machines LLC\ArtCraft Updater' -ErrorAction SilentlyContinue
  Log ("HKLM record: InstallDir=$($reg.InstallDir) Version=$($reg.Version)")
  $arp = Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*' -ErrorAction SilentlyContinue | Where-Object { $_.DisplayName -eq 'ArtCraft Updater' } | Select-Object -First 1
  Log ("Apps and features entry: $($arp.DisplayName) $($arp.DisplayVersion) by $($arp.Publisher); NoModify=$($arp.NoModify) NoRepair=$($arp.NoRepair)")
  $acl = Get-Acl $Dir
  foreach ($a in $acl.Access) { Log ("  ACL {0} {1} {2}" -f $a.IdentityReference, $a.AccessControlType, $a.FileSystemRights) }
  Log ("staging folder present=" + (Test-Path "$Dir\staging"))
  Start-Sleep -Seconds 2
  if (Test-Path "$Dir\updater.log") { Log ("service log: " + ((Get-Content "$Dir\updater.log" -Tail 3) -join ' / ')) }

  Log 'STEP 2: install it again over itself while the service runs (upgrade path)'
  $before = (Service).ProcessId
  $code = Msi @('/i', "`"$UpdaterMsi`"", '/qn', '/norestart') 'lp-updater-reinstall.log'
  [void](WaitFor { (Service).State -eq 'Running' } 30)
  $s = Service
  Log ("msiexec exit=$code; service state=$($s.State) pid before=$before after=$($s.ProcessId) (a new process means it was stopped and started)")

  Log "STEP 3: install the real signed CADCraft per-machine (the old version)"
  $code = Msi @('/i', "`"$MsiFrom`"", '/qn', '/norestart', 'ALLUSERS=1') 'lp-cadcraft-from.log'
  Log ("msiexec exit=$code; installed exe present=" + (Test-Path "$PfApp\cadcraft.exe") + " fileVersion=" + (Get-Item "$PfApp\cadcraft.exe" -ErrorAction SilentlyContinue).VersionInfo.FileVersion)

  Log 'STEP 4: hand over to the normal user'
  New-Item -ItemType File -Path (Join-Path $Root 'lp-ready') -Force | Out-Null
  $answered = WaitFor { Test-Path (Join-Path $Root 'lp-client-done') } 900
  Log "client finished=$answered"
  Log ("after the update: installed exe fileVersion=" + (Get-Item "$PfApp\cadcraft.exe" -ErrorAction SilentlyContinue).VersionInfo.FileVersion)
  if (Test-Path "$Dir\updater.log") { Copy-Item "$Dir\updater.log" (Join-Path $Root 'lp-service-audit.log') -Force }

  Log 'STEP 5: stop the service and ask again as the launcher would'
  & sc.exe stop $Svc 2>&1 | Out-Null
  [void](WaitFor { (Service).State -eq 'Stopped' } 30)
  Log ("service state=" + (Service).State)
  $env:E2E_FROM = ''; $env:E2E_TO = ''
  $out = & $TestExe --ignored --nocapture --test-threads=1 with_no_service 2>&1
  $out | ForEach-Object { Log "  | $_" }
  Log "no-service check exit=$LASTEXITCODE"
}
catch { Log "ERROR: $($_.Exception.Message)" }
finally {
  Log 'STEP 6: CLEANUP'
  $x14 = Msi @('/x', "`"$MsiTo`"", '/qn', '/norestart') 'lp-uninstall-to.log'
  $x13 = Msi @('/x', "`"$MsiFrom`"", '/qn', '/norestart') 'lp-uninstall-from.log'
  Log "CADCraft uninstall exits: new -> $x14 ; old -> $x13  (1605 means that version was not installed)"
  $xu = Msi @('/x', "`"$UpdaterMsi`"", '/qn', '/norestart') 'lp-updater-uninstall.log'
  Log "updater uninstall exit=$xu"
  [void](WaitFor { -not (Service) } 20)
  # Safety net only if the uninstall left anything.
  if (Service) { Log 'service still registered: removing by hand'; & sc.exe stop $Svc 2>&1 | Out-Null; & sc.exe delete $Svc 2>&1 | Out-Null }
  Log ("left behind: service=" + [bool](Service) + " updaterDir=" + (Test-Path $Dir) + " stagingDir=" + (Test-Path "$Dir\staging") +
    " ArtCraftFolder=" + (Test-Path 'C:\Program Files\ArtCraft') + " programFilesCadcraft=" + (Test-Path $PfApp) +
    " hklmRecord=" + (Test-Path 'HKLM:\SOFTWARE\Learning Machines LLC\ArtCraft Updater'))
  if (Test-Path $Dir) { Log 'updater folder remained: removing by hand'; Remove-Item -LiteralPath $Dir -Recurse -Force -ErrorAction SilentlyContinue }
  Log 'finished'
  New-Item -ItemType File -Path (Join-Path $Root 'lp-elevated-finished') -Force | Out-Null
}
