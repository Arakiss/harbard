#!/usr/bin/env python3
"""Exercise replacement failures with isolated bundles and simulated system tools."""
import contextlib
import importlib.util
import json
import os
import pathlib
import shutil
import subprocess
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from unittest import mock

ROOT = pathlib.Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("desktop_runtime", ROOT / "scripts/desktop-runtime.py")
runtime = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runtime)


class DesktopInstall(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="harbard install test ")
        self.base = pathlib.Path(self.temp.name).resolve()
        self.root = self.base / "repo"
        self.desktop = self.root / "apps/desktop"
        self.console = self.root / "apps/console"
        self.bin = self.base / "bin"
        self.primary = self.base / "primary"
        self.mirror = self.base / "mirror"
        for directory in (self.desktop / "scripts", self.console, self.root / "scripts", self.bin, self.primary, self.mirror):
            directory.mkdir(parents=True)
        for path in ("scripts/update-desktop.sh", "scripts/desktop-runtime.py", "apps/desktop/scripts/install.sh"):
            shutil.copy2(ROOT / path, self.root / path)
        for directory, content in ((self.primary, b"old-primary"), (self.mirror, b"old-mirror")):
            exe = directory / "harbard.app/Contents/MacOS/harbard-desktop"
            exe.parent.mkdir(parents=True)
            exe.write_bytes(content)
        bundle = self.desktop / "src-tauri/target/release/bundle/macos/harbard.app/Contents/MacOS/harbard-desktop"
        bundle.parent.mkdir(parents=True)
        bundle.write_bytes(b"new-signed-fixture")
        self.env = {**os.environ, "HOME": str(self.base / "home"), "PATH": str(self.bin) + os.pathsep + os.environ["PATH"], "HARBARD_APP_DIR": str(self.primary), "HARBARD_MIRROR_APP_DIR": str(self.mirror), "HARBARD_REGISTER_APP": "0", "HARBARD_OPEN_APP": "0", "TEST_BASE": str(self.base), "TEST_CONSOLE": str(self.console)}
        self.tool("bunx", "import os,sys,pathlib\nif os.environ.get('TEST_SOURCE_CHANGE'): (pathlib.Path(os.environ['TEST_CONSOLE'])/'changed.ts').write_text('changed during build')\nsys.exit(8 if os.environ.get('TEST_NATIVE_FAIL') else 0)")
        self.tool("codesign", "import os,sys\nsys.exit(7 if os.environ.get('TEST_SIGN_FAIL') else 0)")
        self.tool("ditto", "import os,sys,shutil\nif os.environ.get('TEST_COPY_FAIL') and sys.argv[2].startswith(os.environ['HARBARD_MIRROR_APP_DIR']): sys.exit(8)\nshutil.copytree(sys.argv[1],sys.argv[2],symlinks=True)")
        self.tool("mv", "import os,sys,pathlib,shutil\nif os.environ.get('TEST_SWAP_FAIL') and '.harbard-stage.' in sys.argv[1] and sys.argv[2] == os.environ['HARBARD_MIRROR_APP_DIR']+'/harbard.app': sys.exit(9)\nshutil.move(sys.argv[1],sys.argv[2])")
        self.tool("ps", "import os,sys\nprint(('1' if os.environ.get('TEST_STALE_LISTENER') else '222') if '-o' in sys.argv else os.environ.get('TEST_RUNNING',''))")
        self.tool("lsof", "print('333')")
        self.tool("uname", "print('Darwin')")
        self.tool("curl", "")
        self.tool("bun", "import pathlib\np=pathlib.Path.cwd()/'.next'\np.mkdir(exist_ok=True)\n(p/'BUILD_ID').write_text('fixture-build')\na=p/'static/chunks/app.js'\na.parent.mkdir(parents=True,exist_ok=True)\na.write_bytes(b'fixture-asset')")
        self.tool("launchctl", "import os,sys,pathlib\np=pathlib.Path(os.environ['TEST_BASE'])/'kicked'\nif sys.argv[1]=='kickstart': p.write_text('yes')\nelse: print('working directory = '+os.environ.get('TEST_JOB_CWD',os.environ['TEST_CONSOLE'])+'\\npid = '+('222' if p.exists() else '111'))")
        for name in ("open", "osascript", "pkill"):
            self.tool(name, "import pathlib,os,sys\n(pathlib.Path(os.environ['TEST_BASE'])/'forbidden-ui').write_text('called')\nsys.exit(99)")

    def tearDown(self):
        self.temp.cleanup()

    def tool(self, name, code):
        path = self.bin / name
        path.write_text("#!" + shutil.which("python3") + "\n" + code + "\n")
        path.chmod(0o755)

    def install(self, **extra):
        result = subprocess.run(["/bin/bash", str(self.desktop / "scripts/install.sh")], env={**self.env, **extra}, capture_output=True, text=True)
        self.assertFalse((self.base / "forbidden-ui").exists())
        return result

    def content(self, directory):
        return (directory / "harbard.app/Contents/MacOS/harbard-desktop").read_bytes()

    def assert_old(self):
        self.assertEqual(self.content(self.primary), b"old-primary")
        self.assertEqual(self.content(self.mirror), b"old-mirror")

    def test_success_preserves_both_backups_and_equal_replacements(self):
        result = self.install()
        self.assertEqual(result.returncode, 0, result.stderr)
        for directory, old in ((self.primary, b"old-primary"), (self.mirror, b"old-mirror")):
            self.assertEqual(self.content(directory), b"new-signed-fixture")
            backups = list(directory.glob(".harbard-backup.*/harbard.app/Contents/MacOS/harbard-desktop"))
            self.assertEqual(len(backups), 1)
            self.assertEqual(backups[0].read_bytes(), old)

    def test_sign_failure_leaves_both_incumbents(self):
        self.assertNotEqual(self.install(TEST_SIGN_FAIL="1").returncode, 0)
        self.assert_old()

    def test_mirror_prepare_failure_leaves_both_incumbents(self):
        self.assertNotEqual(self.install(TEST_COPY_FAIL="1").returncode, 0)
        self.assert_old()

    def test_second_swap_failure_restores_both_incumbents(self):
        self.assertNotEqual(self.install(TEST_SWAP_FAIL="1").returncode, 0)
        self.assert_old()

    def test_running_installed_copy_is_refused_without_quitting(self):
        path = self.mirror / "harbard.app/Contents/MacOS/harbard-desktop"
        result = self.install(TEST_RUNNING=str(path))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("is running", result.stderr)
        self.assert_old()

    def init_git(self):
        (self.root / ".gitignore").write_text(".next/\ntarget/\n")
        subprocess.run(["git", "init", "-q", str(self.root)], check=True)
        subprocess.run(["git", "-C", str(self.root), "add", "scripts", "apps/desktop/scripts", ".gitignore"], check=True)
        subprocess.run(["git", "-C", str(self.root), "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-qm", "test: establish isolated fixture"], check=True)

    @contextlib.contextmanager
    def server(self, corrupt=False):
        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                self.send_response(200)
                self.end_headers()
                if self.path.startswith("/_next/"):
                    self.wfile.write(b"wrong-asset" if corrupt else b"fixture-asset")
                else:
                    self.wfile.write(b'<script src="/_next/static/chunks/app.js"></script>')
            def log_message(self, *args):
                pass
        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            yield f"http://127.0.0.1:{server.server_port}"
        finally:
            server.shutdown()
            server.server_close()
            thread.join()

    def update(self, **extra):
        return subprocess.run(["/bin/bash", str(self.root / "scripts/update-desktop.sh")], env={**self.env, **extra}, capture_output=True, text=True)

    def test_update_refuses_foreign_console_before_any_replacement(self):
        self.init_git()
        result = self.update(TEST_JOB_CWD=str(self.base))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("not the production console", result.stderr)
        self.assertFalse((self.base / "kicked").exists())
        self.assert_old()

    def test_native_build_failure_happens_before_restart_or_replacement(self):
        self.init_git()
        result = self.update(TEST_NATIVE_FAIL="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((self.base / "kicked").exists())
        self.assert_old()

    def test_nonlocal_console_url_refuses_before_build_or_replacement(self):
        self.init_git()
        result = self.update(HARBARD_PRODUCTION_CONSOLE_URL="http://example.invalid:3419")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("loopback HTTP origin", result.stderr)
        self.assertFalse((self.console / ".next").exists())
        self.assertFalse((self.base / "kicked").exists())
        self.assert_old()

    def test_source_change_during_build_refuses_before_restart_or_replacement(self):
        self.init_git()
        result = self.update(TEST_SOURCE_CHANGE="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("source changed during", result.stderr)
        self.assertFalse((self.base / "kicked").exists())
        self.assert_old()

    def test_update_receipt_ties_source_build_assets_and_both_installed_copies(self):
        self.init_git()
        with self.server() as url:
            result = self.update(HARBARD_PRODUCTION_CONSOLE_URL=url)
        self.assertEqual(result.returncode, 0, result.stderr)
        receipts = list((self.root / "target/desktop-installs").glob("*.json"))
        self.assertEqual(len(receipts), 1)
        receipt = json.loads(receipts[0].read_text())
        self.assertEqual(receipt["source"]["revision"], runtime.git(self.root, "rev-parse", "HEAD").decode())
        self.assertEqual(receipt["console_build_id"], "fixture-build")
        self.assertEqual(receipt["runtime"]["job_pid"], 222)
        self.assertEqual(len(receipt["runtime"]["assets"]), 1)
        self.assertEqual(len(receipt["installed_apps"]), 2)
        self.assertEqual(receipt["installed_apps"][0]["executable_sha256"], receipt["installed_apps"][1]["executable_sha256"])
        self.assertFalse((self.base / "forbidden-ui").exists())

    def test_stale_listener_cannot_pass_update_readiness(self):
        self.init_git()
        with self.server() as url:
            result = self.update(HARBARD_PRODUCTION_CONSOLE_URL=url, TEST_STALE_LISTENER="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("not descended", result.stderr)

    def test_mismatched_served_asset_cannot_pass_update_readiness(self):
        self.init_git()
        with self.server(corrupt=True) as url:
            result = self.update(HARBARD_PRODUCTION_CONSOLE_URL=url)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("differs from the current build", result.stderr)

    def test_source_fingerprint_detects_tracked_and_untracked_input_changes(self):
        self.init_git()
        first = runtime.source(self.root)
        (self.console / "new-source.ts").write_text("export const value = 1")
        second = runtime.source(self.root)
        self.assertNotEqual(first["input_sha256"], second["input_sha256"])
        (self.root / "scripts/update-desktop.sh").write_text("changed")
        third = runtime.source(self.root)
        self.assertNotEqual(second["input_sha256"], third["input_sha256"])


if __name__ == "__main__":
    unittest.main()
