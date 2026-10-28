# Launcher installation paths

New copies use `%LOCALAPPDATA%\Programs\ArtCraft\Launcher`. Updates restore the existing
registered exact folder, and an explicit `INSTALLFOLDER` wins. The existing UpgradeCode,
per-user scope, account-owned registrations, shortcuts and desktop preference remain intact.

The launcher remains per-user only so its current self-update design is not changed into a
machine-wide deployment. A machine-wide launcher/updater service is separate work, not a
hidden part of this path change. The apps themselves have dual-scope installers.

App updates query Windows Installer registration (`ProductsEx` and `Product.Context`) for the
current account and the machine. A display/uninstall registry hive is not evidence of the
actual context. An unknown, invalid or conflicting context stops the update before download;
it never silently takes a fresh-install default. The query does not use `Win32_Product` or
run any installation/repair action. Native tests cover this stop through the real install entry
point, as well as context mapping, malformed identifiers and query failure.

## ArtCraft Updater (separate installer)

`artcraft-updater.wxs` builds `artcraft-updater-<version>-windows-<arch>.msi`, a **per-machine**
package that installs one Windows service (`ArtCraftUpdater`, LocalSystem) into
`Program Files\ArtCraft\Updater`. It lets the launcher update Craft Apps installed for all users
without a permission prompt for every update. It is not part of the launcher's own installer,
which stays per-user and is untouched. Installing it asks Windows for permission once; the launcher
must explain that first and never run it silently. It runs no installer code (no custom actions),
and its folder cannot be redirected from the command line, because the service refuses to run from
outside Program Files. Build it with `package-updater.ps1`; check a built one with
`check-updater-policy.ps1` (reads the tables, installs nothing). `test-updater-policy.ps1` builds
non-executable fixtures for all three architectures and checks them, and
`mutation-check-updater-policy.py` breaks one rule at a time to prove the check notices.

Run `./packaging/windows/test-install-policy.ps1 -OutputDirectory ./target/install-policy-tests`
on Windows with WiX 5.0.2 and its UI/Util extensions. The x64/x86/arm64 fixtures are deliberately
non-executable. Never install them. They check compiled paths, identity and registration, not
real self-update, elevation, cancellation, rollback or uninstall. Those require an authorised
clean Windows environment and real payloads. Packaging CI and release packaging run the gates.
