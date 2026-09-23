#!/usr/bin/env python3
"""Tests for finalize-signed-hashes.py (AnyChat-style post-sign checksums)."""

from __future__ import annotations

import hashlib
import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
FINALIZE = HERE / "finalize-signed-hashes.py"
ASSEMBLE = REPO / "scripts" / "assemble-official-plugin.sh"


def _digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _write_manifest(root: Path, **platform_extra: object) -> None:
    platforms = {
        "darwin-arm64": {"entrypoint": "bin/darwin-arm64/easybooks", **platform_extra},
        "win32-x64": {"entrypoint": "bin/win32-x64/easybooks.exe", **platform_extra},
    }
    runtime = {
        "version": 3,
        "binary": {
            "name": "easybooks",
            "platforms": platforms,
            "verifyChecksum": True,
        },
        "source_sha": "abc123",
    }
    (root / "runtime-manifest.json").write_text(json.dumps(runtime, indent=2) + "\n")


def _write_bins(root: Path, mac: bytes = b"mac-bin", win: bytes = b"win-bin") -> tuple[str, str]:
    mac_path = root / "bin" / "darwin-arm64" / "easybooks"
    win_path = root / "bin" / "win32-x64" / "easybooks.exe"
    mac_path.parent.mkdir(parents=True, exist_ok=True)
    win_path.parent.mkdir(parents=True, exist_ok=True)
    mac_path.write_bytes(mac)
    win_path.write_bytes(win)
    return _digest(mac), _digest(win)


def _run(args: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(FINALIZE), *args],
        capture_output=True,
        text=True,
    )


class FinalizeSignedHashesTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp(prefix="easybooks-finalize-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)

    def test_write_sidecars_and_manifest_sha256(self) -> None:
        _write_manifest(self.tmp)
        mac_hash, win_hash = _write_bins(self.tmp)
        proc = _run(["--staged", str(self.tmp)])
        self.assertEqual(proc.returncode, 0, proc.stderr)
        runtime = json.loads((self.tmp / "runtime-manifest.json").read_text())
        platforms = runtime["binary"]["platforms"]
        self.assertEqual(platforms["darwin-arm64"]["sha256"], mac_hash)
        self.assertEqual(platforms["win32-x64"]["sha256"], win_hash)
        self.assertEqual(
            (self.tmp / "bin/darwin-arm64/easybooks.sha256").read_text(),
            mac_hash + "\n",
        )
        self.assertEqual(
            (self.tmp / "bin/win32-x64/easybooks.exe.sha256").read_text(),
            win_hash + "\n",
        )
        self.assertEqual(runtime["source_sha"], "abc123")

    def test_idempotent_rewrite_after_resign(self) -> None:
        _write_manifest(self.tmp)
        _write_bins(self.tmp, b"unsigned")
        self.assertEqual(_run(["--staged", str(self.tmp)]).returncode, 0)
        mac_hash, win_hash = _write_bins(self.tmp, b"signed-mac", b"signed-win")
        proc = _run(["--staged", str(self.tmp)])
        self.assertEqual(proc.returncode, 0, proc.stderr)
        runtime = json.loads((self.tmp / "runtime-manifest.json").read_text())
        self.assertEqual(runtime["binary"]["platforms"]["darwin-arm64"]["sha256"], mac_hash)
        self.assertEqual(runtime["binary"]["platforms"]["win32-x64"]["sha256"], win_hash)

    def test_check_accepts_matching_tree(self) -> None:
        _write_manifest(self.tmp)
        _write_bins(self.tmp)
        self.assertEqual(_run(["--staged", str(self.tmp)]).returncode, 0)
        proc = _run(["--staged", str(self.tmp), "--check"])
        self.assertEqual(proc.returncode, 0, proc.stderr)

    def test_check_fails_without_sha256(self) -> None:
        _write_manifest(self.tmp)
        _write_bins(self.tmp)
        proc = _run(["--staged", str(self.tmp), "--check"])
        self.assertEqual(proc.returncode, 2)
        self.assertIn("missing sha256", proc.stderr)

    def test_check_fails_on_sidecar_drift(self) -> None:
        _write_manifest(self.tmp)
        _write_bins(self.tmp)
        self.assertEqual(_run(["--staged", str(self.tmp)]).returncode, 0)
        (self.tmp / "bin/darwin-arm64/easybooks.sha256").write_text("0" * 64 + "\n")
        proc = _run(["--staged", str(self.tmp), "--check"])
        self.assertEqual(proc.returncode, 2)
        self.assertIn("sidecar", proc.stderr)

    def test_assemble_emits_sidecars_and_manifest_hashes(self) -> None:
        stage = self.tmp / "stage"
        out = self.tmp / "plugin"
        (stage / "cli-aarch64-apple-darwin").mkdir(parents=True)
        (stage / "cli-x86_64-pc-windows-gnu").mkdir(parents=True)
        mac = b"assembled-mac"
        win = b"assembled-win"
        (stage / "cli-aarch64-apple-darwin" / "easybooks").write_bytes(mac)
        (stage / "cli-x86_64-pc-windows-gnu" / "easybooks.exe").write_bytes(win)
        proc = subprocess.run(
            ["bash", str(ASSEMBLE), str(stage), str(out)],
            capture_output=True,
            text=True,
        )
        self.assertEqual(proc.returncode, 0, proc.stderr + proc.stdout)
        runtime = json.loads((out / "runtime-manifest.json").read_text())
        platforms = runtime["binary"]["platforms"]
        self.assertEqual(platforms["darwin-arm64"]["sha256"], _digest(mac))
        self.assertEqual(platforms["win32-x64"]["sha256"], _digest(win))
        self.assertTrue((out / "bin/darwin-arm64/easybooks.sha256").is_file())
        self.assertTrue((out / "bin/win32-x64/easybooks.exe.sha256").is_file())
        self.assertTrue(runtime.get("source_sha"))
        check = _run(["--staged", str(out), "--check"])
        self.assertEqual(check.returncode, 0, check.stderr)


if __name__ == "__main__":
    unittest.main()
