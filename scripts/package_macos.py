"""Package a macOS app and a separate executable archive for self_update."""
import os
import plistlib
import shutil
import subprocess
import sys
import tomllib
import zipfile
from pathlib import Path

target = sys.argv[1]
version = tomllib.loads(Path("Cargo.toml").read_text())["package"]["version"]
dist = Path("dist")
dist.mkdir(exist_ok=True)
app = dist / "Roc Request Bin.app"
macos = app / "Contents/MacOS"
resources = app / "Contents/Resources"
macos.mkdir(parents=True, exist_ok=True)
resources.mkdir(parents=True, exist_ok=True)
binary = Path("target") / target / "release/roc-request-bin"
shutil.copy2(binary, macos / binary.name)
shutil.copy2("assets/app.icns", resources / "app.icns")
with (app / "Contents/Info.plist").open("wb") as f:
    plistlib.dump({"CFBundleName": "Roc Request Bin", "CFBundleDisplayName": "Roc Request Bin",
                  "CFBundleIdentifier": "com.roc.request-bin", "CFBundleVersion": version,
                  "CFBundleShortVersionString": version, "CFBundleExecutable": binary.name,
                  "CFBundlePackageType": "APPL", "CFBundleIconFile": "app.icns",
                  "NSHighResolutionCapable": True, "LSMinimumSystemVersion": "12.0"}, f)
subprocess.run(["codesign", "--force", "--deep", "--sign", "-", str(app)], check=True)
architecture = "arm64" if target.startswith("aarch64-") else "amd64"
# APFS is normally case-insensitive: app/update filenames must differ by more
# than letter case, or zip updates the app archive rather than creating a new one.
subprocess.run(["ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", str(app),
                str(dist / f"Roc-Request-Bin-macos-{architecture}.zip")], check=True)
# zip preserves executable mode, allowing self_update to replace the live binary.
subprocess.run(["zip", "-j", str((dist / f"roc-request-bin-{target}.zip").resolve()), str(binary)], check=True)

with zipfile.ZipFile(dist / f"roc-request-bin-{target}.zip") as archive:
    assert archive.namelist() == ["roc-request-bin"], "Update archive must contain only the executable"
    assert archive.getinfo("roc-request-bin").external_attr >> 16 & 0o111, "Missing executable permissions"
with zipfile.ZipFile(dist / f"Roc-Request-Bin-macos-{architecture}.zip") as archive:
    assert "Roc Request Bin.app/Contents/Info.plist" in archive.namelist()
    assert "Roc Request Bin.app/Contents/MacOS/roc-request-bin" in archive.namelist()
    assert "roc-request-bin" not in archive.namelist(), "App and update archives must be separate"
