# Elevated half of the REAL-APP run: a real signed CADCraft 0.13.0 is installed per-machine, the
# real service updates it to the real 0.14.0 from GitHub, then everything is removed again.
# The agent restores the user's own per-user install afterwards (not elevated, as the launcher did).
param(
  [Parameter(Mandatory = $true)][string] $Root,
  [Parameter(Mandatory = $true)][string] $ServiceExe,
  [Parameter(Mandatory = $true)][string] $Msi013,
  [Parameter(Mandatory = $true)][string] $Msi014
)
$ErrorActionPreference = 'Continue'
$Log = Join-Path $Root 'real-results.txt'
$Dir = 'C:\Program Files\ArtCraftUpdaterE2E'
$Svc = 'ArtCraftUpdaterE2E'
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
Remove-Item (Join-Path $Root 'service-ready'), (Join-Path $Root 'client-done'), (Join-Path $Root 'elevated-finished') -Force -ErrorAction SilentlyContinue
Set-Content -Path $Log -Value ''
try {
  $elevated = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
  Log "elevated=$elevated"
  if (-not $elevated) { throw 'not elevated' }
  if (Test-Path "$env:LOCALAPPDATA\Programs\ArtCraft\CADCraft\cadcraft.exe") { Log 'note: a per-user CADCraft is still present (it should have been removed first)' }

  Log 'STEP 1: install the real signed CADCraft 0.13.0 per-machine (ALLUSERS=1)'
  $code = Msi @('/i', "`"$Msi013`"", '/qn', '/norestart', 'ALLUSERS=1') 'real-install-013.log'
  Log "msiexec exit=$code"
  Log ("installed exe present=" + (Test-Path "$PfApp\cadcraft.exe"))
  if (Test-Path "$PfApp\cadcraft.exe") {
    $s = Get-AuthenticodeSignature "$PfApp\cadcraft.exe"
    Log ("exe signature=$($s.Status) $($s.SignerCertificate.Subject) fileVersion=$((Get-Item "$PfApp\cadcraft.exe").VersionInfo.FileVersion)")
  }

  Log 'STEP 2: start the real service as LocalSystem'
  New-Item -ItemType Directory -Path $Dir -Force | Out-Null
  Copy-Item -LiteralPath $ServiceExe -Destination (Join-Path $Dir 'artcraft-updater-service.exe') -Force
  $bin = '"' + $Dir + '\artcraft-updater-service.exe" run'
  [void](New-Service -Name $Svc -BinaryPathName $bin -DisplayName 'ArtCraft Updater (test)' -StartupType Manual)
  [void](& sc.exe start $Svc 2>&1)
  $running = WaitFor { (Get-Service $Svc -ErrorAction SilentlyContinue).Status -eq 'Running' } 30
  Log "service running=$running"
  New-Item -ItemType File -Path (Join-Path $Root 'service-ready') -Force | Out-Null
  $answered = WaitFor { Test-Path (Join-Path $Root 'client-done') } 900
  Log "client finished=$answered"
}
catch { Log "ERROR: $($_.Exception.Message)" }
finally {
  Log 'CLEANUP'
  & sc.exe stop $Svc 2>&1 | Out-Null
  [void](WaitFor { (Get-Service $Svc -ErrorAction SilentlyContinue).Status -ne 'Running' } 30)
  $audit = Join-Path $Dir 'updater.log'
  if (Test-Path $audit) { Copy-Item $audit (Join-Path $Root 'real-service-audit.log') -Force }
  Log ("sc delete: " + ((& sc.exe delete $Svc 2>&1) -join ' '))
  Start-Sleep -Seconds 1
  Remove-Item -LiteralPath $Dir -Recurse -Force -ErrorAction SilentlyContinue
  # Remove whichever per-machine CADCraft is installed (0.14.0 after a successful update, else 0.13.0).
  $x14 = Msi @('/x', "`"$Msi014`"", '/qn', '/norestart') 'real-uninstall-014.log'
  $x13 = Msi @('/x', "`"$Msi013`"", '/qn', '/norestart') 'real-uninstall-013.log'
  Log "uninstall exits: 0.14.0 -> $x14 ; 0.13.0 -> $x13  (1605 means that version was not installed)"
  Log ("left behind: service=" + [bool](Get-Service $Svc -ErrorAction SilentlyContinue) + " serviceDir=" + (Test-Path $Dir) + " programFilesApp=" + (Test-Path $PfApp))
  Log 'finished'
  New-Item -ItemType File -Path (Join-Path $Root 'elevated-finished') -Force | Out-Null
}
