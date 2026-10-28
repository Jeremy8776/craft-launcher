"""Client half of the service end-to-end test. Run NON-elevated, while e2e.ps1 holds the service up.

Checks, as a normal user: the pipe is reachable; the real engine (running as SYSTEM, using real
Windows Installer records) refuses what it should, in plain words; the service folder and its
staging folder cannot be written by a normal user; and the audit log recorded the requests.
"""
import json
import pathlib
import subprocess
import sys

DIR = pathlib.Path(r"C:\Program Files\ArtCraftUpdaterE2E")
EXE = pathlib.Path(sys.argv[1])  # the built test binary running tests/e2e_client.rs
results = []


def check(name, ok, detail=""):
    results.append((name, bool(ok), detail))
    print(("PASS " if ok else "FAIL ") + name + (f"  [{detail}]" if detail else ""), flush=True)


# 1. Not elevated.
groups = subprocess.run([r"C:\Windows\System32\whoami.exe", "/groups"], capture_output=True, text=True).stdout
check("client is NOT elevated", "High Mandatory Level" not in groups and "S-1-16-12288" not in groups)

# 2. Requests over the real pipe (the Rust client in tests/e2e_client.rs prints one JSON line each).
done = subprocess.run([str(EXE), "--ignored", "--nocapture", "--test-threads=1"], capture_output=True, text=True, timeout=120)
replies = [json.loads(l[l.index("REPLY ") + 6:]) for l in done.stdout.splitlines() if "REPLY " in l]
check("client test program ran", done.returncode == 0, done.stdout[-300:] + done.stderr[-300:] if done.returncode else "")
by = {r["label"]: r["reply"] for r in replies}
text = lambda label: json.dumps(by.get(label, {}))
# Every Craft app on a PC that used the launcher has a machine-wide Windows Installer record, so a far-future
# version passes the installed/newer rules and is stopped at the fetch (no such release on GitHub).
check("machine-wide app, far-future version: passes the rules, stopped at fetch", "has no release" in text("not-installed"), text("not-installed"))
check("unknown app refused", "not part of the Craft App Suite" in text("unknown-app"), text("unknown-app"))
check("the launcher itself refused", "not part of the Craft App Suite" in text("launcher"), text("launcher"))
check("garbage refused", "not understood" in text("garbage"), text("garbage"))
check("extra field (a path) refused", "not understood" in text("path-field"), text("path-field"))
check("oversize refused", "too large" in text("oversize"), text("oversize"))
check("service still answers after the bad ones", "has no release" in text("after"), text("after"))

# 3. A normal user can't write where the checks depend on it.
for label, target in (("service folder", DIR), ("staging folder", DIR / "staging")):
    try:
        (target / "planted.msi").write_bytes(b"x")
        (target / "planted.msi").unlink()
        check(f"normal user can NOT write to the {label}", False, "write succeeded")
    except PermissionError:
        check(f"normal user can NOT write to the {label}", True)
    except OSError as e:
        check(f"normal user can NOT write to the {label}", True, f"{type(e).__name__}")
try:
    (DIR / "artcraft-updater-service.exe").write_bytes(b"x")
    check("normal user can NOT overwrite the service exe", False, "write succeeded")
except OSError:
    check("normal user can NOT overwrite the service exe", True)

# 4. A normal user can't stop the service.
stop = subprocess.run([r"C:\Windows\System32\sc.exe", "stop", "ArtCraftUpdaterE2E"], capture_output=True, text=True)
check("normal user can NOT stop the service", "Access is denied" in stop.stdout + stop.stderr or "FAILED 5" in stop.stdout, (stop.stdout + stop.stderr).strip()[:120])

# 5. The audit log saw it.
log = (DIR / "updater.log").read_text(errors="replace") if (DIR / "updater.log").exists() else ""
check("audit log has the started line", "started, staging in" in log)
check("audit log has the refused requests", "cadcraft 9.9.9: refused" in log, log[-300:])

failed = [n for n, ok, _ in results if not ok]
print(f"\n{len(results) - len(failed)}/{len(results)} passed")
pathlib.Path(sys.argv[2]).write_text(json.dumps([{"name": n, "ok": ok, "detail": d} for n, ok, d in results], indent=1))
sys.exit(1 if failed else 0)
