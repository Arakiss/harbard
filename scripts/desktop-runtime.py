#!/usr/bin/env python3
"""Private source/build receipts and bounded production-console verification."""
import datetime
import hashlib
import json
import os
import pathlib
import re
import subprocess
import stat
import sys
import urllib.parse
import urllib.request


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
        if not (name.startswith(("apps/", "assets/", "crates/", "packages/", "scripts/")) or name in ("Cargo.toml", "Cargo.lock", "bun.lock", "package.json", "justfile")):
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


def local_origin(url):
    parsed = urllib.parse.urlsplit(url)
    if parsed.scheme != "http" or parsed.hostname not in ("127.0.0.1", "localhost", "::1") or parsed.path not in ("", "/") or parsed.query or parsed.fragment or parsed.username:
        raise ValueError("production console URL must be a loopback HTTP origin")
    # Evaluate the port here so malformed ports fail before any mutation.
    if parsed.port is not None and parsed.port < 1:
        raise ValueError("production console URL requires a valid TCP port")
    return parsed


def verify(root, url, job_pid):
    parsed = local_origin(url)
    port = parsed.port or 80
    listeners = subprocess.check_output(["lsof", "-nP", f"-iTCP:{port}", "-sTCP:LISTEN", "-t"]).decode().split()
    if not listeners:
        raise ValueError("console has no listening process")
    for listener in set(listeners):
        current = listener
        for _ in range(64):
            if current == str(job_pid):
                break
            if current in ("", "0", "1"):
                raise ValueError("console listener is not descended from the restarted LaunchAgent")
            current = subprocess.check_output(["ps", "-o", "ppid=", "-p", current]).decode().strip()
        else:
            raise ValueError("console listener ancestry exceeded its bound")
    origin = url.rstrip("/")
    pages = {}
    for route in ("/", "/?shell=desktop", "/pulse?shell=desktop"):
        with urllib.request.urlopen(origin + route, timeout=10) as response:
            if response.status != 200:
                raise ValueError("console page did not return HTTP 200")
            pages[route] = response.read().decode(errors="replace")
    assets = set()
    for html in pages.values():
        assets.update(re.findall(r'(?:src|href)="(/_next/static/[^"?]+)', html))
    if not assets:
        raise ValueError("console pages expose no production assets")
    verified = {}
    next_root = (root / "apps/console/.next").resolve()
    for asset in sorted(assets):
        local = (next_root / asset.removeprefix("/_next/")).resolve()
        if next_root not in local.parents or not local.is_file():
            raise ValueError("served console references an asset outside the current build")
        with urllib.request.urlopen(origin + asset, timeout=10) as response:
            data = response.read()
        if data != local.read_bytes():
            raise ValueError("served console asset differs from the current build")
        verified[asset] = hashlib.sha256(data).hexdigest()
    return {"url": origin, "job_pid": int(job_pid), "listener_pids": sorted(map(int, set(listeners))), "assets": verified}


def main():
    if len(sys.argv) == 3 and sys.argv[1] == "validate-url":
        local_origin(sys.argv[2])
        return
    action, root_arg, receipt_arg, *extra = sys.argv[1:]
    root = pathlib.Path(root_arg).resolve()
    receipt = pathlib.Path(receipt_arg)
    if action == "capture":
        data = {"schema": 1, "created_at": datetime.datetime.now(datetime.timezone.utc).isoformat(), "source": source(root)}
    else:
        data = json.loads(receipt.read_text())
        if source(root) != data["source"]:
            raise ValueError("source changed during the build/update; refusing a mixed-revision receipt")
        build_id = (root / "apps/console/.next/BUILD_ID").read_text().strip()
        if not re.fullmatch(r"[A-Za-z0-9_-]+", build_id):
            raise ValueError("missing or invalid console BUILD_ID")
        data["console_build_id"] = build_id
        if action == "verify":
            data["runtime"] = verify(root, extra[0], extra[1])
            app_dir = pathlib.Path(os.environ.get("HARBARD_APP_DIR") or ("/Applications" if os.access("/Applications", os.W_OK) else pathlib.Path.home() / "Applications")).resolve()
            dirs = {app_dir}
            mirror = os.environ.get("HARBARD_MIRROR_APP_DIR")
            if mirror or app_dir == pathlib.Path("/Applications"):
                dirs.add(pathlib.Path(mirror or pathlib.Path.home() / "Applications").resolve())
            data["installed_apps"] = []
            for directory in sorted(dirs):
                app = directory / "harbard.app"
                embedded = json.loads((app / "Contents/Resources/harbard-build.json").read_text())
                if embedded["source"] != data["source"] or embedded["console_build_id"] != build_id:
                    raise ValueError("installed app receipt does not match the console build")
                data["installed_apps"].append({"path": str(app), "executable_sha256": hashlib.sha256((app / "Contents/MacOS/harbard-desktop").read_bytes()).hexdigest()})
        elif action != "built":
            raise ValueError("unknown receipt action")
    receipt.write_text(json.dumps(data, indent=2) + "\n")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"error: desktop runtime verification: {error}", file=sys.stderr)
        sys.exit(1)
