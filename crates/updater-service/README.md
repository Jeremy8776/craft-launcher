# artcraft-launcher-updater-service (spike)

A Windows service that installs Craft App updates for **per-machine** installs without a UAC
prompt for every update.

## Why

A per-machine update today runs `msiexec /i ... ALLUSERS=1`, so Windows asks for permission each
time. With a dozen apps that is a prompt per app. Chrome, Adobe, Firefox and Edge solve this the same
way: one UAC prompt at install, then a SYSTEM service does the updating.

## How a request is handled (`engine.rs`)

The caller sends `{ "app", "version" }` over a local named pipe. Nothing else: no path, URL or
argument field exists, and an extra field is refused.

1. App must be one of the official apps in the launcher catalog (not the launcher itself, and
   not a community app, which the service **declines**: it fetches from `storytold` only).
2. The app must already be installed **per-machine** according to Windows Installer's own
   admin-only records (see below), and the version must be **newer**. The service updates; it never
   installs fresh and never downgrades.
3. Its recorded install folder must be known and inside Program Files. Otherwise the service
   **declines** (a distinct reply, not an error): the launcher carries on with its normal update
   route, which asks Windows for permission, so nobody is blocked. The folder is passed to the
   installer as `INSTALLFOLDER`, so the update stays where the app is.
4. Fetch `https://github.com/<repo>/releases/download/v<version>/` into the staging folder, with
   the launcher's HTTP code and download allowlist (`fetch.rs`).
5. The file must be a regular file inside staging, after resolving links.
6. Its name must be exactly `<app>-<version>-windows-<arch>.msi`.
7. SHA-256 must equal the release's `SHA256SUMS.txt` entry.
8. Authenticode must verify (WinTrust, with revocation checking) and the signer `O=` must be
   exactly `Learning Machines Inc` (`wintrust.rs`, `publisher.rs`).
9. The installer's own `ProductName`, `Manufacturer` and `ProductVersion` (read with the `msi`
   crate, only after the signature is accepted) must be the app's at the requested version
   (`package.rs`). The name, checksum and signature say who made the file, not which product it
   installs.
10. Run a fixed `msiexec /i <file> /qn /norestart /l*v <log> ALLUSERS=1`.

## Where it runs (`service.rs`, `pipe.rs`)

- The service refuses to start unless its exe is inside Program Files, whose default ACL only lets
  SYSTEM, Administrators and TrustedInstaller write. Its `staging` folder inherits that, which is
  what makes "check, then install" safe. It also refuses a `staging` that is a link. The service
  relies on that Windows default and does not read the ACL: an administrator who loosens the
  Program Files ACL turns "check, then install" back into a race.
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
| 30 mutants of those guards, all killed | `mutation-check.py` |
| **Elevated run 1:** the engine's real `msiexec` line updated a renamed per-machine product 1.2.2 to 1.2.3, and a downgrade was refused by Windows Installer (1603); cleaned up | `tests/elevated_install.rs` |
| **Elevated run 2:** the real service ran as `NT AUTHORITY\SYSTEM`; staging inherited the Program Files ACL (Users read-only); a normal user could not write to the service folder, staging or the exe, nor stop the service; refusals came back in plain words; the real machine-wide records and a real HTTPS fetch worked as SYSTEM; the audit log matched; service removed cleanly | `e2e/` |
| **Elevated run 3:** a genuine per-machine install was seen as per-machine by the service's own detection (machine version, component paths), updated in place with `INSTALLFOLDER`, re-registered at the new version, a downgrade refused, and an app outside Program Files declined; cleaned up | `tests/elevated_install.rs` |
| **Elevated run 3:** a temporary standard Windows account (created and deleted by the script) reached the pipe, got plain refusals, and could not write to the service folder, staging or the exe, nor stop the service | `e2e/user_checks.ps1` |
| **Real app, elevated run 4:** real signed CADCraft 0.13.0 installed per-machine, then the real service (SYSTEM) fetched the real v0.14.0 from GitHub over HTTPS, checked hash, signature (with revocation) and publisher, and updated it in place at `C:\Program Files\CADCraft` in 3.4 s with no prompt. Repeat and older requests were refused as not newer. Machine record, file version and exe signature all confirmed 0.14.0 | `e2e/real_update.ps1` |
| **Not run:** the revocation check against a really revoked certificate | no revoked test certificate to hand |
| **Not covered by the non-elevated mutation run:** the `S-1-5-18` machine-slot rule | pinned by the elevated run (a per-machine product must be seen, a per-user one must not) |

## Finding fixed: the registry hive is not the install context

The first version trusted the Add/Remove Programs entry (HKLM or HKCU) to say whether an app was
per-machine. On the development PC every app has an HKLM entry but are recorded by Windows
Installer for one user (`Installer\UserData\S-1-5-21-...`), so the service wrongly treated them as
per-machine. It now asks Windows Installer's own admin-only record: a product is per-machine only if
it is listed under `Installer\UserData\S-1-5-18`. A real-registry test pins this (`machine.rs`).

Consequence for custom folders: a per-user install in a custom folder never reaches the service. It
updates with no UAC prompt through the launcher's per-user route, so nobody is blocked.

## Custom install folders

- **Per-user installs in a custom folder** never reach the service; they update with no UAC prompt
  through the launcher's per-user route.
- **Per-machine installs in Program Files** (default or a subfolder): updated in place by the service.
- **Per-machine installs elsewhere** (for example `D:\Apps`): declined, so the launcher uses its
  normal UAC route. They are not blocked. Updating them silently would let SYSTEM write into a
  folder a normal user may control (a junction swapped in after the check sends the files
  anywhere). Supporting them needs an ACL check that the folder is admin-only; not built.

## The launcher's side

`crates/updater-client` holds what the launcher and the service share: the framing, `Reply`,
`Outcome`, `request_body` (an app id and a version, nothing else) and `ask`, which turns every way
the service can answer or be missing into an `Answer`. It exists as its own crate because this crate
depends on `artcraft-launcher-native`, so `native` cannot depend on this one.

`native::silent_update` is the hook. In `install_at`, only for an MSI update of an app registered
for all users, it asks the service before downloading anything. Only `Answer::Installed` ends the
job; a decline, a refusal, an error or no service at all leaves the update to the normal route
(download, then `msiexec` with Windows' own prompt). Mutation check for that side:
`crates/updater-client/mutation-check.py`.

The pipe's name is fixed, so any local program could create a pipe with that name while the service
is not running and answer "installed". Two checks stop that from skipping an update. First, before
sending anything the launcher asks Windows' service manager which process is the running
`ArtCraftUpdater` service and talks only to that process (`silent_update::is_the_service`). A first
version of this check looked at the program file of the pipe's owner instead; a real run showed a
normal user is not allowed to open the system service's process at all, so every real update would
have quietly taken the prompt route. Second, "installed" is believed only if Windows Installer's own
records, read again by the launcher's detection, show the release's version; otherwise the normal
route runs. A program that holds the pipe open and never answers is rejected at connect, before any
wait.

## Proven with the real installer and the launcher's own install path

`e2e/launcher_path.ps1` (elevated, one UAC prompt) installs the real updater MSI built by
`packaging/windows/package-updater.ps1`, then a real signed per-machine CADCraft 0.13.0.
`e2e/launcher_path_user.ps1` then runs, as a normal user, the ignored test
`crates/native/tests/windows_all_users_update.rs`, which calls the launcher's `install::install`
for CADCraft 0.14.0 with an asset URL that refuses connections. Result of the last run: the service
installed 0.14.0 in 3.3 s; the launcher made no download folder and showed no prompt; Windows
Installer's records, read by the launcher's own detection, said 0.14.0; a normal user could not write
to the service folder, staging or the program, and could not stop the service. With the service
stopped, `ask` answered "unavailable" in under a millisecond (the launcher then takes its normal
route). Uninstalling the updater MSI left no service, folder or registry value.

`e2e/updater_upgrade.ps1` (elevated) covers the updater package itself: 0.9.0 installed, then 0.9.1
installed over the running service (new process, version record and Apps and features entry moved to
0.9.1, one entry only, pipe still there), going back to 0.9.0 refused with the service untouched,
and a clean uninstall (no service, folder, registry value or entry left).

Not covered: the launcher's normal route actually running after a decline (covered by unit tests
only, since it would show a Windows prompt), a signed updater MSI (the builds were unsigned because
no signing secrets are set here), and the revocation check against a really revoked certificate.

## Not built yet

- **Release wiring** for the updater MSI. `packaging/windows/package-updater.ps1` builds, signs and
  checks it, but the release workflow (a shared one in `craft-common-actions`) does not call it yet,
  so it is not published with a release.
- **Launcher side, offer to turn it on:** the launcher already asks the service (see below), but has
  no screen yet that offers "silent updates for all-user installs" and installs the service.
- **Publisher string.** The certificate says `Learning Machines Inc`; version resources and
  registry keys say `Learning Machines LLC`. Confirm which is intended.
- **A second Windows account** to prove the pipe refuses other accounts and that a standard user's
  launcher can use it. (Done in `e2e/`; kept here as a reminder of what that run covered.)

## Gate

```
cargo test   -p artcraft-launcher-updater-service -j 8
cargo clippy -p artcraft-launcher-updater-service --all-targets -j 8 -- -D warnings
cargo fmt    -p artcraft-launcher-updater-service --check
python crates/updater-service/mutation-check.py .      # 30 mutants, all must be killed
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
