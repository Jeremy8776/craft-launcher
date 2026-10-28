"""Mutation check for the launcher side of the updater service (client crate + native hook).
Run exclusively: edits source in place and restores it."""
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(sys.argv[1])
C = ROOT / "crates/updater-client/src"
N = ROOT / "crates/native/src"
S = ROOT / "crates/updater-service/src"

# (file, old, new, label, package to test)
M = [
    (C / "answer.rs", "Ok(Reply::Ran { outcome: Outcome::Installed | Outcome::InstalledRestartNeeded }) => Answer::Installed,",
     "Ok(Reply::Ran { outcome: Outcome::Installed | Outcome::InstalledRestartNeeded | Outcome::Busy }) => Answer::Installed,", "busy counted as installed", "artcraft-launcher-updater-client"),
    (C / "answer.rs", "Ok(Reply::Ran { outcome: Outcome::Failed(code) }) => Answer::Failed(code),",
     "Ok(Reply::Ran { outcome: Outcome::Failed(_) }) => Answer::Installed,", "failed counted as installed", "artcraft-launcher-updater-client"),
    (C / "answer.rs", "Ok(Reply::Declined { reason }) => Answer::Declined(reason),",
     "Ok(Reply::Declined { reason }) => Answer::Refused(reason),", "decline reported as refusal", "artcraft-launcher-updater-client"),
    (C / "answer.rs", "Err(why) => Answer::Unavailable(why),", "Err(_) => Answer::Installed,", "missing service counted as installed", "artcraft-launcher-updater-client"),
    (C / "answer.rs", "matches!(self, Self::Installed)", "true", "is_installed always true", "artcraft-launcher-updater-client"),
    (C / "answer.rs", "Request { app: app_id, version }", "Request { app: version, version: app_id }", "request fields swapped", "artcraft-launcher-updater-client"),
    (N / "silent_update.rs", "is_update && kind == PackageKind::Msi && scope == Some(MsiScope::PerMachine)",
     "kind == PackageKind::Msi && scope == Some(MsiScope::PerMachine)", "fresh install sent to the service", "artcraft-launcher-native"),
    (N / "silent_update.rs", "is_update && kind == PackageKind::Msi && scope == Some(MsiScope::PerMachine)",
     "is_update && kind == PackageKind::Msi", "per-user copy sent to the service", "artcraft-launcher-native"),
    (N / "silent_update.rs", "is_update && kind == PackageKind::Msi && scope == Some(MsiScope::PerMachine)",
     "is_update && scope == Some(MsiScope::PerMachine)", "non-MSI package sent to the service", "artcraft-launcher-native"),
    (N / "silent_update.rs", "    answer.is_installed()\n}", "    true\n}", "any answer counts as done", "artcraft-launcher-native"),
    (N / "silent_update.rs", "service.ask(app.id, &version.to_string())", "service.ask(app.id, &release.tag)", "tag sent instead of version", "artcraft-launcher-native"),
    (N / "silent_update.rs", "    let Some(version) = release.version.as_ref() else {\n        return false;\n    };\n",
     "    let version = release.version.clone().unwrap_or_else(|| artcraft_launcher_core::version::parse(\"0.0.0\").unwrap());\n", "versionless release sent", "artcraft-launcher-native"),
    (S / "wire.rs", "Err(Refusal::Declined(why)) => Reply::Declined { reason: why.clone() },", "", "decline not told apart from a refusal", "artcraft-launcher-updater-service"),
]

results = []
backups = {}
try:
    for path, old, new, label, pkg in M:
        raw = path.read_bytes().decode()
        crlf = "\r\n" in raw
        text = raw.replace("\r\n", "\n")
        if text.count(old) != 1:
            results.append((label, "PATTERN NOT FOUND/UNIQUE"))
            continue
        backups.setdefault(path, raw)
        mutated = text.replace(old, new)
        path.write_bytes((mutated.replace("\n", "\r\n") if crlf else mutated).encode())
        cmd = ["cargo", "test", "-p", pkg, "--lib", "-j", "8"]
        p = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)
        out = p.stdout + p.stderr
        path.write_bytes(raw.encode())
        if p.returncode == 0:
            results.append((label, "SURVIVED"))
        elif "error[E" in out or "could not compile" in out:
            results.append((label, "DID NOT COMPILE: " + next((l for l in out.splitlines() if l.startswith("error")), "")[:90]))
        else:
            failed = sorted({l.split()[1].split("::")[-1] for l in out.splitlines() if l.startswith("test ") and l.endswith("FAILED")})
            results.append((label, "killed by " + ", ".join(failed)[:100]))
finally:
    for path, raw in backups.items():
        path.write_bytes(raw.encode())

for label, status in results:
    print(f"{status[:28]:<28} | {label}")
bad = [r for r in results if not r[1].startswith("killed")]
print("\nsurvivors/problems:", len(bad), "of", len(results))
sys.exit(1 if bad else 0)
