"""Builds two renamed per-machine test MSIs for the updater service end-to-end test.

Product "IptzUpdaterE2E" (never the real UpgradeCode), versions 1.2.2 and 1.2.3, each carrying a
text file that says which version it is. The files are named like real Craft App releases
(cadcraft-<version>-windows-x64.msi) because the engine binds a package to the app and version
asked for. Nothing here is signed.
"""
import pathlib
import subprocess
import sys

OUT = pathlib.Path(sys.argv[1])
WIX = pathlib.Path.home() / ".dotnet" / "tools" / "wix.exe"

WXS = """<Wix xmlns="http://wixtoolset.org/schemas/v4/wxs">
  <Package Name="IptzUpdaterE2E" Manufacturer="Iptz Test" Version="$(var.Version)"
           UpgradeCode="7d1c2a50-3b0e-4c55-9a6a-5f0d6e1b9e21" Scope="perMachine">
    <MajorUpgrade DowngradeErrorMessage="A newer IptzUpdaterE2E is already installed." />
    <MediaTemplate EmbedCab="yes" />
    <StandardDirectory Id="ProgramFiles64Folder">
      <Directory Id="INSTALLFOLDER" Name="IptzUpdaterE2E">
        <Component Id="Marker" Guid="3a8f5d2e-91c4-4e7b-8d0a-6b2f7c1e4a90">
          <File Id="VersionTxt" Source="$(var.Payload)" Name="version.txt" KeyPath="yes" />
        </Component>
      </Directory>
    </StandardDirectory>
    <Feature Id="Main"><ComponentRef Id="Marker" /></Feature>
  </Package>
</Wix>
"""

OUT.mkdir(parents=True, exist_ok=True)
(OUT / "product.wxs").write_text(WXS, encoding="utf-8")
for version in ("1.2.2", "1.2.3"):
    folder = OUT / f"v{version.replace('.', '')}"
    folder.mkdir(exist_ok=True)
    payload = folder / "version.txt"
    payload.write_text(f"IptzUpdaterE2E {version}\n", encoding="utf-8")
    msi = folder / f"cadcraft-{version}-windows-x64.msi"
    cmd = [str(WIX), "build", "-arch", "x64", "-d", f"Version={version}", "-d", f"Payload={payload}", "-o", str(msi), str(OUT / "product.wxs")]
    done = subprocess.run(cmd, capture_output=True, text=True)
    print(version, "->", msi, "OK" if done.returncode == 0 else f"FAILED\n{done.stdout}\n{done.stderr}")
    if done.returncode != 0:
        sys.exit(1)
