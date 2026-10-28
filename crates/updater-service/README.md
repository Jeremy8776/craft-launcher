# artcraft-launcher-updater-service (spike)

A Windows service that installs Craft App updates for **per-machine** installs without a UAC
prompt for every update.

## Why

A per-machine update today runs `msiexec /i ... ALLUSERS=1`, so Windows asks for permission each
time. With 13 apps that is a prompt per app. Chrome, Adobe, Firefox and Edge solve this the same
way: one UAC prompt at install, then a SYSTEM service does the updating.

## How a request is handled (`engine.rs`)

The caller sends `{ "app", "version" }` over a local named pipe. Nothing else: no path, URL or
argument field exists, and an extra field is refused.

1. App must be one of the 13 in the launcher catalog (not the launcher itself).
2. The app must already be installed **per-machine** (Windows Installer records, through the
   launcher's own detection code) and the version must be **newer**. The service updates; it never
   installs fresh and never downgrades.
3. Fetch `https://github.com/<repo>/releases/download/v<version>/` into the staging folder, with
   the launcher's HTTP code and download allowlist (`fetch.rs`).
4. The file must be a regular file inside staging, after resolving links.
5. Its name must be exactly `<app>-<version>-windows-<arch>.msi`.
6. SHA-256 must equal the release's `SHA256SUMS.txt` entry.
7. Authenticode must verify (WinTrust, with revocation checking) and the signer `O=` must be
   exactly `Learning Machines Inc` (`wintrust.rs`, `publisher.rs`).
8. Run a fixed `msiexec /i <file> /qn /norestart /l*v <log> ALLUSERS=1`.

## Where it runs (`service.rs`, `pipe.rs`)

- The service refuses to start unless its exe is inside Program Files, whose default ACL only lets
  SYSTEM, Administrators and TrustedInstaller write. Its `staging` folder inherits that, which is
  what makes "check, then install" safe. It also refuses a `staging` that is a link.
- The pipe is local only, with the descriptor `D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;IU)`:
  SYSTEM and Administrators full, interactive users read/write, nobody else.
- Each connection is read on its own thread with a deadline, so a silent client can't hold up
  the next one. Installs run one at a time.
- `SignedCaller` (checks the connecting exe is signed by us) exists but is **not** used:
  `verifysign` documents PID checks as unreliable, so it would be defence in depth only. The
  engine is the security boundary.
- Every request and result is appended to `updater.log` beside the exe.

## Status

Built and tested, **not yet run as a real service or against a real install on a PC**:

| What | Evidence |
|---|---|
| Engine, request parsing, staging, publisher match, fixed command line, exit codes | unit tests |
| Real Authenticode on a Microsoft file, an unsigned file, a signed Craft exe, a tampered copy | `tests/wintrust.rs` |
| Fetcher against a scripted loopback server (good, tampered body, 404, no installer, allowlist) | `fetch/tests.rs` |
| Real named pipe: round trip, refusals, silent client, half frame, oversize, one at a time | `pipe/tests.rs` |
| Startup rules: Program Files only, staging not a link | `service/tests.rs` |
| 25 mutants of those guards, all killed | `mutation-check.py` |
| **Not run:** the service under SYSTEM, the pipe ACL against a normal user, a real per-machine update | `e2e/` (needs one elevated run) |

## Not built yet

- **Per-machine launcher MSI** that installs the service into Program Files and registers it
  (WiX `ServiceInstall`, `LocalSystem`). The launcher MSI is per-user today.
- **Launcher side:** call `pipe::request` for per-machine updates when the service is present, and
  fall back to the existing `msiexec` route (with its UAC prompt) when it is not.
- **Publisher string.** The certificate says `Learning Machines Inc`; version resources and
  registry keys say `Learning Machines LLC`. Confirm which is intended.
- **A second Windows account** to prove the pipe refuses other accounts and that a standard user's
  launcher can use it.

## Gate

```
cargo test   -p artcraft-launcher-updater-service -j 8
cargo clippy -p artcraft-launcher-updater-service --all-targets -j 8 -- -D warnings
cargo fmt    -p artcraft-launcher-updater-service --check
python crates/updater-service/mutation-check.py .      # 25 mutants, all must be killed
```

## End-to-end run (needs one elevated prompt)

`e2e/e2e.ps1` runs elevated once and does two things, cleaning up in a `finally`:

1. `tests/elevated_install.rs` (ignored by default): the engine's real command line updates a
   renamed per-machine test product 1.2.2 to 1.2.3, then a downgrade is attempted, then it is
   removed. MSIs come from `e2e/build_msis.py` (WiX 5).
2. The real service binary is registered as `ArtCraftUpdaterE2E` (LocalSystem) in a test folder
   under Program Files. While it runs, `e2e/e2e_client.py` (not elevated) talks to it over the
   pipe, checks it refuses bad requests in plain words, checks a normal user can't write to its
   folder or staging folder or stop it, and reads the audit log.
