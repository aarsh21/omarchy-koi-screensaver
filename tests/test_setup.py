"""Installer contracts in a network-isolated, read-only-host Bubblewrap sandbox.

Package installation, compilation, and HTTP are simulated. Extraction, installation,
symlink changes, hooks, and removal use the real scripts and filesystem tools.
"""

import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import textwrap
import unittest


ROOT = Path(__file__).resolve().parents[1]
BIN = Path("/usr/local/bin/koi-screensaver")
STATE = Path("/usr/local/lib/koi-screensaver")
SESSION = Path("/usr/share/omarchy/bin/omarchy-screensaver")
STOCK = "/usr/bin/omarchy-screensaver"


class SetupTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if not shutil.which("bwrap") or not Path(STOCK).is_file():
            raise unittest.SkipTest("requires Omarchy and Bubblewrap")
        probe = subprocess.run(
            ["bwrap", "--ro-bind", "/", "/", "--unshare-user", "--unshare-net", "true"],
            capture_output=True,
        )
        if probe.returncode:
            raise unittest.SkipTest("unprivileged Bubblewrap namespaces are unavailable")

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="koi-setup-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        for directory in ("bin", "local/bin", "session", "home", "log", "tmp with spaces", "project"):
            (self.root / directory).mkdir(parents=True)
        self.original = "../../../bin/omarchy-screensaver"
        (self.root / "session/omarchy-screensaver").symlink_to(self.original)
        (self.root / "fixture-binary").write_text("#!/bin/sh\nprintf 'koi-screensaver test\\n'\n")
        (self.root / "fixture-binary").chmod(0o755)
        for relative in ("install.sh", "Cargo.toml", "Cargo.lock", "scripts/koi-screensaver.hook"):
            destination = self.root / "project" / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / relative, destination)
        with tarfile.open(self.root / "source.tar.gz", "w:gz") as archive:
            archive.add(self.root / "project", arcname="source")
        self.command("sudo", """
            if [[ ${1:-} == -n ]]; then
              shift
              [[ ${KOI_TEST_DENY_SUDO:-0} == 0 ]] || exit 1
            fi
            printf '%s\\n' "$*" >> /mnt/log/sudo
            exec "$@"
        """)
        self.command("curl", """
            printf '%s\\n' "$*" >> /mnt/log/curl
            [[ ${KOI_TEST_DOWNLOAD_FAIL:-0} == 0 ]] || exit 22
            while [[ $# -gt 0 ]]; do
              if [[ $1 == --output ]]; then
                cp /mnt/source.tar.gz "$2"
                exit
              fi
              shift
            done
            exit 2
        """)
        self.command("cargo", """
            if [[ $1 == --version ]]; then
              [[ ${KOI_TEST_MISSING_RUST:-0} == 0 || -e /mnt/log/deps-installed ]] || exit 1
              echo 'cargo test'
              exit
            fi
            printf '%s\\n' "$*" >> /mnt/log/cargo
            [[ ${KOI_TEST_BUILD_FAIL:-0} == 0 ]] || exit 9
            while [[ $# -gt 0 ]]; do
              if [[ $1 == --target-dir ]]; then
                mkdir -p "$2/release"
                install -m755 /mnt/fixture-binary "$2/release/koi-screensaver"
                exit
              fi
              shift
            done
            exit 2
        """)
        self.command("rustc", "echo 'rustc test'")
        self.command("pacman", """
            printf '%s\\n' "$*" >> /mnt/log/pacman
            touch /mnt/log/deps-installed
        """)
        self.command("omarchy-launch-screensaver", "exit 0")
        self.command("omarchy-notification-send", 'printf "%s\\n" "$*" >> /mnt/log/notification')
        self.stock_hash = hashlib.sha256(Path(STOCK).read_bytes()).hexdigest()

    def command(self, name, body):
        path = self.root / "bin" / name
        path.write_text("#!/bin/bash\nset -euo pipefail\n" + textwrap.dedent(body).strip() + "\n")
        path.chmod(0o755)

    def run_script(self, args=(), *, env=None, command=None, root=False):
        invocation = [
            "bwrap", "--ro-bind", "/", "/", "--unshare-user", "--unshare-net",
            "--uid", str(0 if root else os.getuid()), "--gid", str(0 if root else os.getgid()),
            "--dev", "/dev", "--bind", str(self.root), "/mnt", "--tmpfs", "/tmp", "--tmpfs", "/home",
            "--bind", str(self.root / "home"), "/home/koi user",
            "--bind", str(self.root / "local"), "/usr/local",
            "--bind", str(self.root / "session"), "/usr/share/omarchy/bin",
            "--setenv", "HOME", "/home/koi user", "--setenv", "PATH", "/mnt/bin:/usr/bin:/bin",
            "--setenv", "TMPDIR", "/mnt/tmp with spaces",
            "--setenv", "CARGO_TARGET_DIR", "/mnt/not-the-install-target",
            "--chdir", "/mnt",
        ]
        for key, value in (env or {}).items():
            invocation += ["--setenv", key, value]
        if command is None:
            invocation += ["bash", "-s", "--", *args]
            source = (ROOT / "setup.sh").read_text()
        else:
            invocation += command
            source = None
        result = subprocess.run(invocation, input=source, text=True, capture_output=True, timeout=30)
        self.assertEqual(hashlib.sha256(Path(STOCK).read_bytes()).hexdigest(), self.stock_hash)
        self.assertEqual(list((self.root / "tmp with spaces").iterdir()), [], result.stderr)
        return result

    def installed(self):
        self.assertEqual((self.root / "local/bin/omarchy-screensaver").readlink(), BIN)
        self.assertEqual((self.root / "session/omarchy-screensaver").readlink(), BIN)
        self.assertEqual((self.root / "local/lib/koi-screensaver/stock-shim-target").read_text().strip(), self.original)
        self.assertTrue((self.root / "local/lib/koi-screensaver/install.sh").is_file())
        self.assertTrue((self.root / "home/.config/omarchy/hooks/post-update.d/koi-screensaver.hook").is_file())
        self.assertEqual((self.root / "local/bin/koi-screensaver").read_bytes(), (self.root / "fixture-binary").read_bytes())

    def test_streamed_setup_reinstall_and_offline_uninstall(self):
        for _ in range(2):
            result = self.run_script()
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.installed()
        self.assertIn("--locked --release --target-dir", (self.root / "log/cargo").read_text())
        result = self.run_script(command=["bash", str(STATE / "install.sh"), "uninstall"])
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual((self.root / "session/omarchy-screensaver").readlink().as_posix(), self.original)
        self.assertFalse((self.root / "local/bin/koi-screensaver").exists())
        self.assertFalse((self.root / "local/lib/koi-screensaver").exists())
        self.assertFalse((self.root / "home/.config/omarchy/hooks/post-update.d/koi-screensaver.hook").exists())
        self.assertEqual(self.run_script(["uninstall"]).returncode, 0)

    def test_failed_update_leaves_the_installed_screensaver_intact(self):
        self.assertEqual(self.run_script().returncode, 0)
        result = self.run_script(env={"KOI_TEST_BUILD_FAIL": "1"})
        self.assertEqual(result.returncode, 9, result.stdout + result.stderr)
        self.installed()

    def test_missing_native_libraries_and_rust_are_installed(self):
        self.command("pkg-config", "exit 1")
        result = self.run_script(env={"KOI_TEST_MISSING_RUST": "1"})
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        packages = (self.root / "log/pacman").read_text().split()
        self.assertEqual(set(packages), {"-S", "--needed", "--noconfirm", "sdl3", "cairo", "rust"})
        self.installed()

    def test_rustup_is_used_without_changing_the_default_toolchain(self):
        self.command("rustup", 'printf "%s\\n" "$*" > /mnt/log/rustup; touch /mnt/log/deps-installed')
        result = self.run_script(env={"KOI_TEST_MISSING_RUST": "1"})
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual((self.root / "log/rustup").read_text().strip(), "toolchain install stable --profile minimal")
        self.installed()

    def test_foreign_login_shim_and_unsupported_layout_are_not_overwritten(self):
        foreign = self.root / "local/bin/omarchy-screensaver"
        foreign.write_text("user-owned screensaver")
        result = self.run_script()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(foreign.read_text(), "user-owned screensaver")
        foreign.unlink()
        session = self.root / "session/omarchy-screensaver"
        session.unlink()
        session.write_text("legacy installation")
        result = self.run_script()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(session.read_text(), "legacy installation")
        self.assertFalse((self.root / "log/sudo").exists())

    def test_failed_download_and_invalid_invocations_do_not_install(self):
        for args, env, root, expected in [
            ([], {"KOI_TEST_DOWNLOAD_FAIL": "1"}, False, 22),
            (["wrong"], {}, False, 2),
            (["install", "extra"], {}, False, 2),
            ([], {}, True, 1),
            (["--help"], {}, False, 0),
        ]:
            with self.subTest(args=args, env=env, root=root):
                result = self.run_script(args, env=env, root=root)
                self.assertEqual(result.returncode, expected, result.stdout + result.stderr)
                self.assertFalse((self.root / "log/sudo").exists())
                self.assertEqual((self.root / "session/omarchy-screensaver").readlink().as_posix(), self.original)

    def test_uninstall_preserves_a_later_replacement(self):
        self.assertEqual(self.run_script().returncode, 0)
        for link in (self.root / "session/omarchy-screensaver", self.root / "local/bin/omarchy-screensaver"):
            link.unlink()
            link.symlink_to("/usr/bin/true")
        result = self.run_script(command=["bash", str(STATE / "install.sh"), "uninstall"])
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual((self.root / "session/omarchy-screensaver").readlink().as_posix(), "/usr/bin/true")
        self.assertEqual((self.root / "local/bin/omarchy-screensaver").readlink().as_posix(), "/usr/bin/true")

    def test_post_update_hook_restores_the_shim_or_notifies(self):
        self.assertEqual(self.run_script().returncode, 0)
        session = self.root / "session/omarchy-screensaver"
        session.unlink()
        session.symlink_to(STOCK)
        hook = "/home/koi user/.config/omarchy/hooks/post-update.d/koi-screensaver.hook"
        self.assertEqual(self.run_script(command=["bash", hook], env={"KOI_TEST_DENY_SUDO": "1"}).returncode, 0)
        self.assertEqual(session.readlink().as_posix(), STOCK)
        self.assertIn("project README", (self.root / "log/notification").read_text())
        self.assertEqual(self.run_script(command=["bash", hook]).returncode, 0)
        self.assertEqual(session.readlink(), BIN)


if __name__ == "__main__":
    unittest.main()
