<#
.SYNOPSIS
  Check the compiled ArtCraft Updater installer without installing it.
.DESCRIPTION
  Reads the MSI's tables only (OpenDatabase, read-only). Nothing is installed, started or written.
  The updater runs as the system, so these are the things that must not drift: it is per-machine,
  it installs one program inside Program Files, the folder cannot be redirected from the command
  line, the service is the one the launcher talks to and runs as the system with the one argument
  the program waits for, it is removed and stopped on uninstall, and no installer code runs.
#>
param([Parameter(Mandatory = $true)] [string] $MsiPath)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$policy = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot 'updater-install-policy.json') | ConvertFrom-Json

function Read-Rows($Database, [string] $Sql, [int] $Columns) {
  try { $view = $Database.OpenView($Sql) } catch { return }   # a table the package does not have
  try {
    [void]$view.Execute()
    while ($null -ne ($record = $view.Fetch())) {
      try {
        $values = @(for ($i = 1; $i -le $Columns; $i++) {
          $record.GetType().InvokeMember('StringData', 'GetProperty', $null, $record, @($i))
        })
        ,$values
      } finally { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($record) }
    }
  } finally {
    [void]$view.Close()
    [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($view)
  }
}

$installer = $null; $database = $null
try {
  $installer = New-Object -ComObject WindowsInstaller.Installer
  $file = (Resolve-Path -LiteralPath $MsiPath).Path
  $database = $installer.OpenDatabase($file, 0)

  # Per-machine, with its own identity.
  $properties = @{}
  foreach ($row in (Read-Rows $database 'SELECT `Property`, `Value` FROM `Property`' 2)) { $properties[$row[0]] = $row[1] }
  if ($properties['ALLUSERS'] -ne '1') { throw 'The updater must install for all users (ALLUSERS=1).' }
  if ($properties.ContainsKey('MSIINSTALLPERUSER')) { throw 'The updater must never install per-user.' }
  if ($properties['UpgradeCode'].Trim('{}') -ne $policy.upgradeCode) { throw 'The updater upgrade identity changed.' }
  if ($properties['UpgradeCode'].Trim('{}') -eq '1AC08DBF-D4CF-4BA4-A81E-CAA5C1004974') { throw 'The updater must not share the launcher upgrade identity.' }
  if (@(Read-Rows $database 'SELECT `Action` FROM `InstallExecuteSequence` WHERE `Action` = ''RemoveExistingProducts''' 1).Count -ne 1) {
    throw 'A newer updater must replace an older one (major upgrade: RemoveExistingProducts).'
  }

  # Inside Program Files, and not a folder a command line can redirect.
  $dirs = @{}
  foreach ($row in (Read-Rows $database 'SELECT `Directory`, `Directory_Parent`, `DefaultDir` FROM `Directory`' 3)) { $dirs[$row[0]] = @($row[1], $row[2]) }
  if (-not ($dirs.ContainsKey('UpdaterFolder') -and $dirs['UpdaterFolder'][0] -eq 'CraftSuiteFolder' -and $dirs['UpdaterFolder'][1] -eq 'Updater')) {
    throw 'The updater folder must be ArtCraft\Updater.'
  }
  if ($dirs['CraftSuiteFolder'][1] -ne 'ArtCraft') { throw 'Expected the updater under the ArtCraft suite folder.' }
  $root = $dirs['CraftSuiteFolder'][0]
  if ($root -notin 'ProgramFiles6432Folder', 'ProgramFilesFolder', 'ProgramFiles64Folder') {
    throw 'The updater must install inside Program Files, using the native folder, not a fixed path.'
  }
  $public = @($dirs.Keys | Where-Object { $_ -cmatch '^[A-Z0-9_.]+$' -and $_ -ne 'TARGETDIR' })
  if ($public.Count) { throw "Folder ids with only capitals can be set from the command line and move the service: $($public -join ', ')" }

  # One program, one service, run as the system.
  $files = @(Read-Rows $database 'SELECT `File`, `FileName` FROM `File`' 2)
  if ($files.Count -ne 1 -or ($files[0][1] -split '\|')[-1] -ne 'artcraft-updater-service.exe') { throw 'The updater must install exactly artcraft-updater-service.exe.' }
  $services = @(Read-Rows $database 'SELECT `Name`, `ServiceType`, `StartType`, `ErrorControl`, `StartName`, `Arguments` FROM `ServiceInstall`' 6)
  if ($services.Count -ne 1) { throw 'Expected exactly one service.' }
  $s = $services[0]
  if ($s[0] -ne $policy.service) { throw 'The launcher talks to the service called ArtCraftUpdater; the name must not change.' }
  if ([int]$s[1] -ne 0x10) { throw 'The service must run in its own process.' }
  if ([int]$s[2] -ne 2) { throw 'The service must start automatically.' }
  if (([int]$s[3] -band 0x7FFF) -ne 1) { throw 'The service must report errors as normal.' }
  if ($s[4] -ne 'LocalSystem') { throw "The service must run as LocalSystem, not $($s[4])." }
  if ($s[5] -ne 'run') { throw 'The program waits for the single argument "run".' }
  $controls = @(Read-Rows $database 'SELECT `Name`, `Event`, `Wait` FROM `ServiceControl`' 3)
  if ($controls.Count -ne 1 -or $controls[0][0] -ne $policy.service) { throw 'Expected one control row for the updater service.' }
  $events = [int]$controls[0][1]
  if (-not ($events -band 0x01)) { throw 'The service must start when installed.' }
  if (-not ($events -band 0x02)) { throw 'The service must stop before its files are replaced.' }
  if (-not ($events -band 0x20)) { throw 'The service must stop when removed.' }
  if (-not ($events -band 0x80)) { throw 'The service must be deleted when removed.' }

  # Nothing else runs, and nothing is written for one user.
  $actions = @(Read-Rows $database 'SELECT `Action`, `Type` FROM `CustomAction`' 2)
  if ($actions.Count) { throw "No installer code may run during setup: $(($actions | ForEach-Object { $_[0] }) -join ', ')" }
  $registry = @(Read-Rows $database 'SELECT `Root`, `Key`, `Name` FROM `Registry`' 3)
  if (@($registry | Where-Object { $_[0] -ne '2' }).Count) { throw 'The updater may write only under the machine (HKLM).' }
  if (@(Read-Rows $database 'SELECT `Shortcut` FROM `Shortcut`' 1).Count) { throw 'The updater has no shortcuts.' }

  Write-Output "PASS $($policy.name): all-users, inside Program Files, folder not redirectable, one LocalSystem service with the launcher's name, stopped and removed on uninstall, no custom actions. No installation performed."
} finally {
  foreach ($com in @($database, $installer)) {
    if ($null -ne $com) { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($com) }
  }
}
