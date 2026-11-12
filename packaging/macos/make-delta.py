"""Build a BSDIFF40 update from two signed MyAICut app bundles."""

import argparse
import filecmp
import hashlib
import json
import plistlib
from pathlib import Path
import re
import stat
import subprocess
from tempfile import TemporaryDirectory
from zipfile import ZIP_STORED, ZipFile

import bsdiff4

EXECUTABLE = "Contents/MacOS/MyAICut"
APP_ID = "com.myaicut.app"
MAX_BUNDLE_SIZE = 256 * 1024 * 1024


def app_version(bundle):
    subprocess.run(["codesign", "--verify", "--strict", "--deep", bundle], check=True)
    with (bundle / "Contents/Info.plist").open("rb") as file:
        info = plistlib.load(file)
    version = info["CFBundleShortVersionString"]
    if info["CFBundleIdentifier"] != APP_ID or not re.fullmatch(r"\d+\.\d+\.\d+", version):
        raise ValueError("Expected a signed stable MyAICut app")
    return version


def bundle_files(bundle):
    files = {}
    for file in (bundle / "Contents").rglob("*"):
        mode = file.lstat().st_mode
        if stat.S_ISDIR(mode):
            continue
        if not stat.S_ISREG(mode):
            raise ValueError(f"Unsupported bundle entry: {file}")
        path = file.relative_to(bundle).as_posix()
        if len(path.encode()) > 240 or any(ord(char) < 32 or char in "\\:" for char in path):
            raise ValueError(f"Unsupported bundle path: {file}")
        files[path] = file
    if EXECUTABLE not in files or len(files) > 513 or sum(file.stat().st_size for file in files.values()) > MAX_BUNDLE_SIZE:
        raise ValueError("Unsupported app bundle layout or size")
    return files


def make_delta(previous, current, full, arch):
    base_version = app_version(previous)
    version = app_version(current)
    if tuple(map(int, base_version.split("."))) >= tuple(map(int, version.split("."))):
        raise ValueError("The previous app must have an older version")
    old = bundle_files(previous)
    new = bundle_files(current)
    with old[EXECUTABLE].open("rb") as file:
        base_hash = hashlib.file_digest(file, "sha256").hexdigest()
    name = f"myaicut-{version}-macos-{arch}-from-{base_version}-{base_hash[:16]}-delta.zip"
    files = sorted(new.keys() - {EXECUTABLE})
    with TemporaryDirectory(dir=full.parent) as temporary:
        patch = Path(temporary) / "executable.bsdiff"
        bsdiff4.file_diff(str(old[EXECUTABLE]), str(new[EXECUTABLE]), str(patch))
        archive = Path(temporary) / name
        with ZipFile(archive, "w", compression=ZIP_STORED) as zip_file:
            zip_file.writestr("manifest.json", json.dumps({"base_sha256": base_hash, "files": files}, separators=(",", ":")))
            zip_file.write(patch, patch.name)
            for path in files:
                before, after = old.get(path), new[path]
                if before is None or before.stat().st_mode != after.stat().st_mode or not filecmp.cmp(before, after, shallow=False):
                    zip_file.write(after, f"files/{path}")
        if archive.stat().st_size >= full.stat().st_size:
            raise ValueError("Delta is not smaller than the full update")
        size = archive.stat().st_size
        archive.replace(full.parent / name)
    print(f"{name} ({size:,} bytes, full update {full.stat().st_size:,} bytes)")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("previous", type=Path)
    parser.add_argument("current", type=Path)
    parser.add_argument("full_update", type=Path)
    parser.add_argument("arch", choices=["aarch64", "x86_64"])
    args = parser.parse_args()
    make_delta(args.previous, args.current, args.full_update, args.arch)


if __name__ == "__main__":
    main()
