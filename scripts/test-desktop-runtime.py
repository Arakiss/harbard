#!/usr/bin/env python3
"""Exercise the desktop build, install and update with isolated bundles and
simulated system tools: never cargo, codesign, launchctl or /Applications."""
import importlib.util
import json
import os
import pathlib
import plistlib
import shutil
import subprocess
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("desktop_runtime", ROOT / "scripts/desktop-runtime.py")
runtime = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runtime)

SCRIPTS = (
    "scripts/update-desktop.sh",
    "scripts/desktop-runtime.py",
    "scripts/retire-console.sh",
    "apps/desktop/scripts/install.sh",
    "apps/desktop/scripts/bundle.sh",
    "apps/desktop/bundle/Info.plist",
)
CONSOLE_PLIST = "Library/LaunchAgents/com.harbard.console.plist"


class DesktopInstall(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="harbard install test ")
        self.base = pathlib.Path(self.temp.name).resolve()
        self.root = self.base / "repo"
        self.desktop = self.root / "apps/desktop"
        self.target = self.desktop / "target"
        self.bin = self.base / "bin"
        self.home = self.base / "home"
        self.primary = self.base / "primary"
        self.mirror = self.base / "mirror"
        for directory in (self.desktop / "scripts", self.desktop / "bundle", self.desktop / "src", self.root / "scripts", self.bin, self.home, self.primary, self.mirror):
            directory.mkdir(parents=True, exist_ok=True)
        for path in SCRIPTS:
            shutil.copy2(ROOT / path, self.root / path)
        (self.desktop / "bundle/icon.icns").write_bytes(b"icns-fixture")
        (self.desktop / "bundle/Assets.car").write_bytes(b"car-fixture")
        (self.desktop / "src/main.rs").write_text("fn main() {}\n")
        for directory, content in ((self.primary, b"old-primary"), (self.mirror, b"old-mirror")):
            exe = directory / "harbard.app/Contents/MacOS/harbard-desktop"
            exe.parent.mkdir(parents=True)
            exe.write_bytes(content)
        self.env = {
            **os.environ,
            "HOME": str(self.home),
            "TMPDIR": str(self.base),
            "PATH": str(self.bin) + os.pathsep + os.environ["PATH"],
            "HARBARD_APP_DIR": str(self.primary),
            "HARBARD_MIRROR_APP_DIR": str(self.mirror),
            "HARBARD_REGISTER_APP": "0",
            "HARBARD_OPEN_APP": "0",
            "TEST_BASE": str(self.base),
            "TEST_TARGET": str(self.target),
            "TEST_SRC": str(self.desktop / "src"),
        }
        for key in ("CARGO_TARGET_DIR", "HARBARD_APP_BUNDLE", "HARBARD_BUILD_RECEIPT", "HARBARD_DESKTOP_SKIP_BUILD", "HARBARD_QUIT_APP"):
            self.env.pop(key, None)
        self.tool("cargo", "\n".join((
            "import json,os,sys,pathlib",
            "t=pathlib.Path(os.environ['TEST_TARGET'])",
            "if sys.argv[1]=='--version': print('cargo 1.99.1 (fixture)')",
            "elif sys.argv[1]=='metadata': print(json.dumps({'target_directory':str(t),'packages':[{'name':'harbard-app','version':'0.3.0-test'}]}))",
            "elif sys.argv[1]=='build':",
            "    pathlib.Path(os.environ['TEST_BASE'],'built').write_text(' '.join(sys.argv[1:]))",
            "    if os.environ.get('TEST_SOURCE_CHANGE'): (pathlib.Path(os.environ['TEST_SRC'])/'changed.rs').write_text('// changed during build')",
            "    if os.environ.get('TEST_NATIVE_FAIL'): sys.exit(8)",
            "    b=t/'release/harbard-app'; b.parent.mkdir(parents=True,exist_ok=True); b.write_bytes(b'new-signed-fixture'); b.chmod(0o755)",
            "else: sys.exit(64)",
        )))
        self.tool("codesign", "import os,sys\nsys.exit(7 if os.environ.get('TEST_SIGN_FAIL') else 0)")
        self.tool("ditto", "import os,sys,shutil\nif os.environ.get('TEST_COPY_FAIL') and sys.argv[2].startswith(os.environ['HARBARD_MIRROR_APP_DIR']): sys.exit(8)\nshutil.copytree(sys.argv[1],sys.argv[2],symlinks=True)")
        self.tool("mv", "import os,sys,pathlib,shutil\nif os.environ.get('TEST_SWAP_FAIL') and '.harbard-stage.' in sys.argv[1] and sys.argv[2] == os.environ['HARBARD_MIRROR_APP_DIR']+'/harbard.app': sys.exit(9)\nshutil.move(sys.argv[1],sys.argv[2])")
        self.tool("ps", "import os\nprint(os.environ.get('TEST_RUNNING',''))")
        self.tool("uname", "print('Darwin')")
        self.tool("launchctl", "\n".join((
            "import os,sys,pathlib",
            "log=pathlib.Path(os.environ['TEST_BASE'])/'launchctl.log'",
            "log.open('a').write(' '.join(sys.argv[1:])+'\\n')",
            "if sys.argv[1]=='print': sys.exit(0 if os.environ.get('TEST_CONSOLE_LOADED') and sys.argv[2].endswith('/com.harbard.console') else 113)",
            "if sys.argv[1]!='bootout': sys.exit(99)",
        )))
        for name in ("open", "osascript", "pkill", "bunx", "bun"):
            self.tool(name, "import pathlib,os,sys\n(pathlib.Path(os.environ['TEST_BASE'])/'forbidden').write_text(sys.argv[0])\nsys.exit(99)")

    def tearDown(self):
        self.temp.cleanup()

    def tool(self, name, code):
        path = self.bin / name
        path.write_text("#!" + shutil.which("python3") + "\n" + code + "\n")
        path.chmod(0o755)

    def run_script(self, path, **extra):
        result = subprocess.run(["/bin/bash", str(path)], env={**self.env, **extra}, capture_output=True, text=True)
        self.assertFalse((self.base / "forbidden").exists(), result.stderr)
        return result

    def fixture_bundle(self):
        bundle = self.base / "built/harbard.app"
        exe = bundle / "Contents/MacOS/harbard-desktop"
        exe.parent.mkdir(parents=True)
        exe.write_bytes(b"new-signed-fixture")
        return bundle

    def install(self, **extra):
        extra = {"HARBARD_DESKTOP_SKIP_BUILD": "1", "HARBARD_APP_BUNDLE": str(self.fixture_bundle()), **extra}
        return self.run_script(self.desktop / "scripts/install.sh", **extra)

    def update(self, **extra):
        return self.run_script(self.root / "scripts/update-desktop.sh", **extra)

    def content(self, directory):
        return (directory / "harbard.app/Contents/MacOS/harbard-desktop").read_bytes()

    def assert_old(self):
        self.assertEqual(self.content(self.primary), b"old-primary")
        self.assertEqual(self.content(self.mirror), b"old-mirror")

    def launchctl(self):
        log = self.base / "launchctl.log"
        return log.read_text().splitlines() if log.exists() else []

    def console_agent(self, loaded=True):
        plist = self.home / CONSOLE_PLIST
        plist.parent.mkdir(parents=True)
        plist.write_text("<plist/>")
        if loaded:
            self.env["TEST_CONSOLE_LOADED"] = "1"
        return plist

    def init_git(self):
        (self.root / ".gitignore").write_text("target/\n")
        subprocess.run(["git", "init", "-q", str(self.root)], check=True)
        subprocess.run(["git", "-C", str(self.root), "add", "-A"], check=True)
        subprocess.run(["git", "-C", str(self.root), "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-qm", "test: establish isolated fixture"], check=True)

    # --- install.sh -------------------------------------------------------

    def test_success_preserves_both_backups_and_equal_replacements(self):
        result = self.install()
        self.assertEqual(result.returncode, 0, result.stderr)
        for directory, old in ((self.primary, b"old-primary"), (self.mirror, b"old-mirror")):
            self.assertEqual(self.content(directory), b"new-signed-fixture")
            backups = list(directory.glob(".harbard-backup.*/harbard.app/Contents/MacOS/harbard-desktop"))
            self.assertEqual(len(backups), 1)
            self.assertEqual(backups[0].read_bytes(), old)

    def test_skip_build_without_a_bundle_is_refused(self):
        result = self.run_script(self.desktop / "scripts/install.sh", HARBARD_DESKTOP_SKIP_BUILD="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("needs HARBARD_APP_BUNDLE", result.stderr)
        self.assertFalse((self.base / "built").exists())
        self.assert_old()

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

    # --- bundle.sh --------------------------------------------------------

    def test_bundle_lays_out_the_app_with_the_cargo_version(self):
        self.init_git()
        result = self.run_script(self.desktop / "scripts/bundle.sh")
        self.assertEqual(result.returncode, 0, result.stderr)
        app = pathlib.Path(result.stdout.strip().splitlines()[-1])
        self.assertEqual(app, self.target / "release/bundle/macos/harbard.app")
        self.assertEqual((app / "Contents/MacOS/harbard-desktop").read_bytes(), b"new-signed-fixture")
        info = plistlib.loads((app / "Contents/Info.plist").read_bytes())
        self.assertEqual(info["CFBundleIdentifier"], "com.harbard.desktop")
        self.assertEqual(info["CFBundleExecutable"], "harbard-desktop")
        self.assertEqual(info["CFBundleShortVersionString"], "0.3.0-test")
        self.assertEqual(info["CFBundleVersion"], "0.3.0-test")
        self.assertEqual(info["CFBundleIconName"], "harbard")
        for name in ("icon.icns", "Assets.car", "harbard-build.json"):
            self.assertTrue((app / "Contents/Resources" / name).is_file(), name)
        embedded = json.loads((app / "Contents/Resources/harbard-build.json").read_text())
        self.assertEqual(embedded["app_version"], "0.3.0-test")
        self.assertIn("--release --locked", (self.base / "built").read_text())

    # --- update-desktop.sh ------------------------------------------------

    def test_build_failure_happens_before_replacement_or_retirement(self):
        self.init_git()
        plist = self.console_agent()
        result = self.update(TEST_NATIVE_FAIL="1")
        self.assertNotEqual(result.returncode, 0)
        self.assert_old()
        self.assertTrue(plist.exists())
        self.assertNotIn("bootout", " ".join(self.launchctl()))

    def test_source_change_during_build_refuses_before_replacement(self):
        self.init_git()
        result = self.update(TEST_SOURCE_CHANGE="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("source changed during", result.stderr)
        self.assert_old()

    def test_running_app_stops_the_update_and_keeps_the_console(self):
        self.init_git()
        plist = self.console_agent()
        result = self.update(TEST_RUNNING=str(self.primary / "harbard.app/Contents/MacOS/harbard-desktop"))
        self.assertNotEqual(result.returncode, 0)
        self.assert_old()
        self.assertTrue(plist.exists())

    def test_update_receipt_ties_source_build_and_both_installed_copies(self):
        self.init_git()
        result = self.update()
        self.assertEqual(result.returncode, 0, result.stderr)
        receipts = list((self.root / "target/desktop-installs").glob("*.json"))
        self.assertEqual(len(receipts), 1)
        receipt = json.loads(receipts[0].read_text())
        self.assertEqual(receipt["source"]["revision"], runtime.git(self.root, "rev-parse", "HEAD").decode())
        self.assertEqual(receipt["app_version"], "0.3.0-test")
        self.assertEqual(len(receipt["installed_apps"]), 2)
        self.assertEqual(receipt["installed_apps"][0]["executable_sha256"], receipt["installed_apps"][1]["executable_sha256"])
        for directory in (self.primary, self.mirror):
            self.assertEqual(self.content(directory), b"new-signed-fixture")
            info = plistlib.loads((directory / "harbard.app/Contents/Info.plist").read_bytes())
            self.assertEqual(info["CFBundleVersion"], "0.3.0-test")

    def test_update_retires_the_console_agent_once_the_app_is_in(self):
        self.init_git()
        plist = self.console_agent()
        result = self.update()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(plist.exists())
        self.assertIn(f"bootout gui/{os.getuid()}/com.harbard.console", self.launchctl())
        self.assertIn("web console is retired", result.stdout)

    def test_update_on_a_mac_without_the_console_touches_no_agent(self):
        self.init_git()
        result = self.update()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("bootout", " ".join(self.launchctl()))

    def test_verify_refuses_an_installed_copy_from_another_build(self):
        self.init_git()
        self.assertEqual(self.update().returncode, 0)
        receipt = next((self.root / "target/desktop-installs").glob("*.json"))
        embedded = self.mirror / "harbard.app/Contents/Resources/harbard-build.json"
        data = json.loads(embedded.read_text())
        data["app_version"] = "0.0.1"
        embedded.write_text(json.dumps(data))
        result = subprocess.run(["python3", str(self.root / "scripts/desktop-runtime.py"), "verify", str(self.root), str(receipt)], env=self.env, capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("does not carry this build", result.stderr)

    def test_source_fingerprint_detects_tracked_and_untracked_input_changes(self):
        self.init_git()
        first = runtime.source(self.root)
        (self.desktop / "src/new_source.rs").write_text("pub const VALUE: u8 = 1;")
        second = runtime.source(self.root)
        self.assertNotEqual(first["input_sha256"], second["input_sha256"])
        (self.root / "scripts/update-desktop.sh").write_text("changed")
        third = runtime.source(self.root)
        self.assertNotEqual(second["input_sha256"], third["input_sha256"])
        (self.root / "notes.md").write_text("not an input")
        self.assertEqual(third["input_sha256"], runtime.source(self.root)["input_sha256"])


if __name__ == "__main__":
    unittest.main()
