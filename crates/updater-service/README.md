# artcraft-launcher-updater-service (spike)

The engine of a Windows service that installs Craft App updates for **per-machine** installs
without a UAC prompt for every update.

## Why

A per-machine update today runs `msiexec /i ... ALLUSERS=1`, so Windows asks for permission each
time. With 13 apps that is a prompt per app. Chrome, Adobe, Firefox and Edge solve this the same
way: one UAC prompt at install, then a SYSTEM service does the updating.

## What this crate is

The part that decides whether to run an installer. It is pure where it can be and refuses by
default. Order of checks (`engine.rs`):

1. Parse the request: only `{ "app", "version" }`. No path, URL or argument field exists.
2. App must be one of the 13 in the launcher catalog (not the launcher itself).
3. Fetch the release into the service's staging folder (trait; real fetcher not written yet).
4. The file must be a regular file inside staging, after resolving links.
5. Its name must be exactly `<app>-<version>-windows-<arch>.msi` for the request.
6. SHA-256 must equal the release's `SHA256SUMS.txt` entry.
7. Authenticode must verify (WinTrust) and the signer `O=` must be exactly `Learning Machines Inc`.
8. Run a fixed `msiexec /i <file> /qn /norestart /l*v <log> ALLUSERS=1`.

## Not built yet (needs decisions or an elevated test run)

- **Service shell** (`windows-service` crate, safe API) and its registration.
- **Authenticated pipe.** ACL it to interactive users, and check the client's exe with
  `verifysign::verify_pid`. Needs a safe pipe crate, which is a dependency-trust call.
- **Real fetcher.** Reuse the native crate's HTTP, `download_url_under` allowlist and
  `parse_sha256sums`. Keep the service from adding a second download path.
- **Per-machine launcher MSI** that creates the service and the staging folder with a
  SYSTEM and Administrators only ACL. The checks above are only safe with that ACL, so the
  service must refuse to start without it.
- **Publisher string.** The cert says `Learning Machines Inc`; version resources and registry
  keys say `Learning Machines LLC`. Confirm which is intended.
- **Downgrades.** Windows Installer refuses them (1603) for MajorUpgrade packages; not re-checked here.

## Gate

```
cargo test  -p artcraft-launcher-updater-service -j 8
cargo clippy -p artcraft-launcher-updater-service --all-targets -j 8 -- -D warnings
cargo fmt   -p artcraft-launcher-updater-service --check
python crates/updater-service/mutation-check.py .      # 14 mutants, all must be killed
```

`tests/wintrust.rs` runs real WinTrust checks on a Microsoft-signed system file, an unsigned
file, and (when installed) a signed Craft exe, plus a copy of it with one byte changed. No
installer is ever run by the tests.
