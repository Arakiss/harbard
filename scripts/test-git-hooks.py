#!/usr/bin/env python3
"""Verify the local gate is reachable in primary and linked Git checkouts."""
import os
import pathlib
import shutil
import subprocess
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parent.parent
JUST = shutil.which("just")


class GitHooks(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="harbard hook test ")
        self.base = pathlib.Path(self.temp.name).resolve()
        self.repo = self.base / "repo"
        self.repo.mkdir()
        self.env = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
        self.env.update({"GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_NOSYSTEM": "1"})
        self.run_command("git", "init", "-q")
        (self.repo / "scripts").mkdir()
        for name in ("justfile", "scripts/pre-push"):
            shutil.copy2(ROOT / name, self.repo / name)
        self.run_command("git", "add", "justfile", "scripts/pre-push")
        self.run_command("git", "-c", "user.name=Hook Test", "-c", "user.email=hook@example.invalid", "commit", "-qm", "test: fixture")

    def tearDown(self):
        self.temp.cleanup()

    def run_command(self, *args, cwd=None, **kwargs):
        return subprocess.run(args, cwd=cwd or self.repo, env=self.env, check=True, capture_output=True, text=True, **kwargs)

    def install(self, checkout):
        self.run_command(JUST, "install-hooks", cwd=checkout)
        gitdir = pathlib.Path(self.run_command("git", "rev-parse", "--absolute-git-dir", cwd=checkout).stdout.strip())
        hook = gitdir / "hooks/pre-push"
        self.assertEqual(hook.read_bytes(), (ROOT / "scripts/pre-push").read_bytes())
        self.assertTrue(os.access(hook, os.X_OK))
        return hook

    def test_primary_and_idempotent_install_preserves_foreign_hook(self):
        foreign = self.repo / ".git/hooks/pre-commit"
        foreign.write_text("foreign hook remains\n")
        self.install(self.repo)
        self.install(self.repo)
        self.assertEqual(foreign.read_text(), "foreign hook remains\n")

    def test_linked_worktree_uses_its_actual_git_directory(self):
        checkout = self.base / "linked checkout"
        self.run_command("git", "worktree", "add", "-qb", "linked", str(checkout))
        self.assertTrue((checkout / ".git").is_file())
        self.install(checkout)
        self.assertFalse((self.repo / ".git/hooks/pre-push").exists())

    def test_worktree_push_blocks_failed_gate_without_mutating_its_repository(self):
        checkout = self.base / "linked checkout"
        self.run_command("git", "worktree", "add", "-qb", "linked", str(checkout))
        hook = self.install(checkout)
        remote = self.base / "remote.git"
        scratch = self.base / "gate scratch.git"
        self.run_command("git", "init", "-q", "--bare", str(remote))
        # A gate may run real Git in another directory. Hook-local GIT_DIR must
        # not turn that fixture init into a reinitialization of the caller.
        import shlex
        (checkout / "justfile").write_text(
            "ci:\n    git init -q --bare " + shlex.quote(str(scratch)) + "\n    false\n"
        )
        before = self.run_command("git", "rev-parse", "HEAD", cwd=checkout).stdout.strip()
        result = subprocess.run(
            ["git", "-c", "core.hooksPath=" + str(hook.parent), "push", str(remote), "HEAD:refs/heads/linked"],
            cwd=checkout, env=self.env, text=True, capture_output=True,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("gate failed", result.stderr)
        self.assertEqual(self.run_command("git", "config", "--get", "core.bare", cwd=checkout).stdout.strip(), "false")
        self.assertEqual(self.run_command("git", "rev-parse", "HEAD", cwd=checkout).stdout.strip(), before)
        self.assertEqual(self.run_command("git", "--git-dir=" + str(scratch), "rev-parse", "--is-bare-repository").stdout.strip(), "true")
        self.assertEqual(self.run_command("git", "--git-dir=" + str(remote), "for-each-ref").stdout.strip(), "")



if __name__ == "__main__":
    unittest.main()
