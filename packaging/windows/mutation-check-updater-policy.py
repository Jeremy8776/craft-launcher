"""Mutation check for check-updater-policy.ps1: break the updater installer one rule at a time,
build the (non-installable) MSI, and expect the policy check to FAIL. Nothing is installed.

    python packaging/windows/mutation-check-updater-policy.py <scratch folder>

Needs WiX 5 on PATH and Windows PowerShell. The source .wxs is never edited; mutants are built
from copies in the scratch folder.
"""
import pathlib
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[1]
WORK = pathlib.Path(sys.argv[1]).resolve()
SOURCE = (HERE / "artcraft-updater.wxs").read_text(encoding="utf-8")

# (old text, new text, rule that should be enforced)
MUTANTS = [
    ('Scope="perMachine"', 'Scope="perUser"', "per-machine"),
    ('Account="LocalSystem"', 'Account="NT AUTHORITY\\LocalService"', "runs as LocalSystem"),
    ('Arguments="run"', 'Arguments="run --anything"', "the one argument"),
    ('Start="auto"', 'Start="demand"', "starts automatically"),
    ('Name="ArtCraftUpdater" DisplayName', 'Name="SomethingElse" DisplayName', "service name the launcher uses"),
    ('Start="install" Stop="both" Remove="uninstall"', 'Start="install" Stop="both"', "removed on uninstall"),
    ('Start="install" Stop="both" Remove="uninstall"', 'Remove="uninstall"', "started when installed"),
    ('UpgradeCode="2EC794AB-99ED-42A4-BE38-98F0A2945B2A"', 'UpgradeCode="1AC08DBF-D4CF-4BA4-A81E-CAA5C1004974"', "identity apart from the launcher"),
    ('<StandardDirectory Id="ProgramFiles6432Folder">', '<StandardDirectory Id="LocalAppDataFolder">', "inside Program Files"),
    ('Root="HKLM" Key="Software\\Learning Machines LLC\\ArtCraft Updater" Name="Version"', 'Root="HKCU" Key="Software\\Learning Machines LLC\\ArtCraft Updater" Name="Version"', "no per-user registry"),
    ("UpdaterFolder", "UPDATERFOLDER", "folder not redirectable from the command line"),
    ('<Feature Id="Main"', '<CustomAction Id="RunSomething" Directory="UpdaterFolder" ExeCommand="cmd.exe /c exit" Execute="deferred" Impersonate="no" Return="ignore" />\n    <InstallExecuteSequence><Custom Action="RunSomething" After="InstallFiles" /></InstallExecuteSequence>\n    <Feature Id="Main"', "no installer code"),
    # No mutant for <MajorUpgrade>: WiX 5 schedules RemoveExistingProducts without it, so removing the
    # element changes nothing a package can observe (the policy still checks the action is there).
]


def build_and_check(name, text):
    folder = WORK / name
    folder.mkdir(parents=True, exist_ok=True)
    (folder / "artcraft-updater-service.exe").write_text("NON-EXECUTABLE MSI METADATA TEST FIXTURE", encoding="ascii")
    wxs = folder / "artcraft-updater.wxs"
    wxs.write_text(text, encoding="utf-8")
    msi = folder / "mutant.msi"
    built = subprocess.run(
        ["wix", "build", str(wxs), "-arch", "x64", "-d", "Version=0.0.1", "-d", f"BinDir={folder}",
         "-d", f"IconPath={ROOT / 'assets/app-icon/artcraft-launcher.ico'}", "-o", str(msi)],
        capture_output=True, text=True)
    if built.returncode != 0:
        return "DID NOT BUILD", (built.stdout + built.stderr).strip().splitlines()[-1][:100]
    checked = subprocess.run(
        ["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(HERE / "check-updater-policy.ps1"), "-MsiPath", str(msi)],
        capture_output=True, text=True)
    first = next((l for l in (checked.stdout + checked.stderr).splitlines() if l.strip()), "")
    return ("killed" if checked.returncode != 0 else "SURVIVED"), first[:100]


results = []
status, why = build_and_check("control", SOURCE)
print(f"control (unmutated) -> {'passes' if status == 'SURVIVED' else status + ': ' + why}")
if status != "SURVIVED":
    sys.exit("the unmutated installer must pass the policy first")
for i, (old, new, rule) in enumerate(MUTANTS):
    if SOURCE.count(old) < 1:
        results.append((rule, "PATTERN NOT FOUND", ""))
        continue
    status, why = build_and_check(f"m{i}", SOURCE.replace(old, new))
    results.append((rule, status, why))
for rule, status, why in results:
    print(f"{status:<16} | {rule:<50} | {why}")
bad = [r for r in results if r[1] != "killed"]
print("\nsurvivors/problems:", len(bad), "of", len(results))
sys.exit(1 if bad else 0)
