# Elevated: upgrade of the updater MSI over a RUNNING service, a refused downgrade, and a clean uninstall.
# Started once, elevated (one UAC prompt). Everything is undone in `finally`.
param(
  [Parameter(Mandatory = $true)][string] $Root,
  [Parameter(Mandatory = $true)][string] $MsiOld,
  [Parameter(Mandatory = $true)][string] $MsiNew
)
$ErrorActionPreference = 'Continue'
$Log = Join-Path $Root 'up-results.txt'
$Dir = 'C:\Program Files\ArtCraft\Updater'
$Svc = 'ArtCraftUpdater'
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
function Arp() { @(Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*' -ErrorAction SilentlyContinue | Where-Object { $_.DisplayName -eq 'ArtCraft Updater' }) }
function State([string] $label) {
  $s = Service
  $reg = Get-ItemProperty 'HKLM:\SOFTWARE\Learning Machines LLC\ArtCraft Updater' -ErrorAction SilentlyContinue
  Log ("$label : service state=$($s.State) pid=$($s.ProcessId) startMode=$($s.StartMode) account=$($s.StartName); HKLM Version=$($reg.Version); Apps-and-features entries=$((Arp).Count) " +
    "($((Arp | ForEach-Object { $_.DisplayVersion }) -join ',')); pipe=" + (Test-Path '\\.\pipe\artcraft-updater') + "; files=" + ((Get-ChildItem $Dir -Recurse -File -ErrorAction SilentlyContinue | ForEach-Object { $_.Name }) -join ','))
}
Set-Content -Path $Log -Value ''
try {
  $elevated = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
  Log "elevated=$elevated"
  if (-not $elevated) { throw 'not elevated' }
  if (Service) { throw 'an ArtCraftUpdater service already exists; not touching it' }

  Log 'STEP 1: install the old updater (0.9.0)'
  Log ("msiexec exit=" + (Msi @('/i', "`"$MsiOld`"", '/qn', '/norestart') 'up-install-old.log'))
  [void](WaitFor { (Service).State -eq 'Running' } 30)
  State 'after install old'
  $pidOld = (Service).ProcessId

  Log 'STEP 2: install the NEW updater (0.9.1) over the running service'
  Log ("msiexec exit=" + (Msi @('/i', "`"$MsiNew`"", '/qn', '/norestart') 'up-install-new.log'))
  [void](WaitFor { (Service).State -eq 'Running' } 30)
  State 'after upgrade'
  Log ("pid before=$pidOld after=$((Service).ProcessId) (different means it was stopped and started)")

  Log 'STEP 3: try to go back to the old version (must be refused)'
  Log ("msiexec exit=" + (Msi @('/i', "`"$MsiOld`"", '/qn', '/norestart') 'up-downgrade.log') + " (1638 = another version is installed; any non-zero is a refusal)")
  State 'after refused downgrade'

  Log 'STEP 4: the service answers after the upgrade'
  $env:E2E_FROM = ''
  $pipeOk = Test-Path '\\.\pipe\artcraft-updater'
  Log "pipe present=$pipeOk"
}
catch { Log "ERROR: $($_.Exception.Message)" }
finally {
  Log 'STEP 5: CLEANUP'
  $xn = Msi @('/x', "`"$MsiNew`"", '/qn', '/norestart') 'up-uninstall-new.log'
  $xo = Msi @('/x', "`"$MsiOld`"", '/qn', '/norestart') 'up-uninstall-old.log'
  Log "uninstall exits: new -> $xn ; old -> $xo (1605 = not installed)"
  [void](WaitFor { -not (Service) } 20)
  if (Service) { Log 'service still registered: removing by hand'; & sc.exe stop $Svc 2>&1 | Out-Null; & sc.exe delete $Svc 2>&1 | Out-Null }
  if (Test-Path $Dir) { Log 'folder remained: removing by hand'; Remove-Item -LiteralPath $Dir -Recurse -Force -ErrorAction SilentlyContinue }
  Log ("left behind: service=" + [bool](Service) + " updaterDir=" + (Test-Path $Dir) + " ArtCraftFolder=" + (Test-Path 'C:\Program Files\ArtCraft') +
    " hklmRecord=" + (Test-Path 'HKLM:\SOFTWARE\Learning Machines LLC\ArtCraft Updater') + " arpEntries=" + (Arp).Count)
  Log 'finished'
  New-Item -ItemType File -Path (Join-Path $Root 'up-finished') -Force | Out-Null
}
