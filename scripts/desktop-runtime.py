#!/usr/bin/env python3
"""Private source receipts for the desktop app: what a build came from, and
proof that every installed copy carries that build.

  capture <root> <receipt>            fingerprint the source before a build
  built   <root> <receipt> <version>  refuse a source that changed during the build
  verify  <root> <receipt>            tie each installed harbard.app to the receipt
"""
import datetime
import hashlib
import json
import os
import pathlib
import re
import subprocess
import stat
import sys

# What the app is built from. Anything else in the checkout cannot change it.
INPUT_DIRS = ("apps/", "assets/", "crates/", "scripts/")
INPUT_FILES = ("Cargo.toml", "Cargo.lock", "justfile", "rust-toolchain.toml")
VERSION = re.compile(r"[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?")


def git(root, *args):
    return subprocess.check_output(["git", "-C", str(root), *args]).strip()


def source(root):
    paths = set(git(root, "ls-files", "-z").split(b"\0"))
    paths.update(git(root, "ls-files", "--others", "--exclude-standard", "-z").split(b"\0"))
    digest = hashlib.sha256()
    count = 0
    for raw in sorted(paths):
        if not raw:
            continue
        name = raw.decode()
        if not (name.startswith(INPUT_DIRS) or name in INPUT_FILES):
            continue
        path = root / name
        digest.update(raw + b"\0")
        if path.exists() or path.is_symlink():
            digest.update(str(stat.S_IMODE(path.lstat().st_mode)).encode() + b"\0")
        if path.is_symlink():
            digest.update(b"link\0" + str(path.readlink()).encode())
        elif path.is_file():
            digest.update(b"file\0" + path.read_bytes())
        else:
            digest.update(b"missing\0")
        digest.update(b"\0")
        count += 1
    return {
        "revision": git(root, "rev-parse", "HEAD").decode(),
        "tree": git(root, "rev-parse", "HEAD^{tree}").decode(),
        "dirty": bool(git(root, "status", "--porcelain", "--untracked-files=normal")),
        "input_sha256": digest.hexdigest(),
        "input_files": count,
    }


def app_dirs():
    """Where apps/desktop/scripts/install.sh puts the app, by the same rules."""
    app_dir = pathlib.Path(os.environ.get("HARBARD_APP_DIR") or ("/Applications" if os.access("/Applications", os.W_OK) else pathlib.Path.home() / "Applications")).resolve()
    dirs = {app_dir}
    mirror = os.environ.get("HARBARD_MIRROR_APP_DIR")
    if mirror or app_dir == pathlib.Path("/Applications"):
        dirs.add(pathlib.Path(mirror or pathlib.Path.home() / "Applications").resolve())
    return sorted(dirs)


def installed(data):
    apps = []
    for directory in app_dirs():
        app = directory / "harbard.app"
        embedded = json.loads((app / "Contents/Resources/harbard-build.json").read_text())
        if embedded.get("source") != data["source"] or embedded.get("app_version") != data["app_version"]:
            raise ValueError(f"{app} does not carry this build")
        apps.append({"path": str(app), "executable_sha256": hashlib.sha256((app / "Contents/MacOS/harbard-desktop").read_bytes()).hexdigest()})
    if len({app["executable_sha256"] for app in apps}) > 1:
        raise ValueError("the installed copies of the app differ")
    return apps


def main():
    action, root_arg, receipt_arg, *extra = sys.argv[1:]
    root = pathlib.Path(root_arg).resolve()
    receipt = pathlib.Path(receipt_arg)
    if action == "capture":
        data = {"schema": 2, "created_at": datetime.datetime.now(datetime.timezone.utc).isoformat(), "source": source(root)}
    else:
        data = json.loads(receipt.read_text())
        if source(root) != data["source"]:
            raise ValueError("source changed during the build/update; refusing a mixed-revision receipt")
        if action == "built":
            if len(extra) != 1 or not VERSION.fullmatch(extra[0]):
                raise ValueError("built needs the app's version")
            data["app_version"] = extra[0]
        elif action == "verify":
            if "app_version" not in data:
                raise ValueError("the receipt has no build")
            data["installed_apps"] = installed(data)
        else:
            raise ValueError("unknown receipt action")
    receipt.write_text(json.dumps(data, indent=2) + "\n")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f"error: desktop runtime verification: {error}", file=sys.stderr)
        sys.exit(1)
