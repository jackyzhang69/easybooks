#!/usr/bin/env python3
"""Assemble must keep sealed STAGE sidecars and stamp runtime-manifest sha256."""

from __future__ import annotations

import hashlib
import json
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path


HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
ASSEMBLE = REPO / "scripts" / "assemble-official-plugin.sh"


def _digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _write_stage(
    stage: Path,
    mac: bytes = b"signed-mac",
    win: bytes = b"signed-win",
    *,
    sidecars: bool = True,
    sidecar_style: str = "shasum",
    corrupt: str | None = None,
    omit: str | None = None,
) -> tuple[str, str]:
    mac_dir = stage / "cli-aarch64-apple-darwin"
    win_dir = stage / "cli-x86_64-pc-windows-gnu"
    mac_dir.mkdir(parents=True)
    win_dir.mkdir(parents=True)
    (mac_dir / "easybooks").write_bytes(mac)
    (win_dir / "easybooks.exe").write_bytes(win)
    mac_hash, win_hash = _digest(mac), _digest(win)
    if not sidecars:
        return mac_hash, win_hash
    mac_text = f"{mac_hash}  cli-aarch64-apple-darwin/easybooks\n" if sidecar_style == "shasum" else mac_hash + "\n"
    win_text = f"{win_hash}  cli-x86_64-pc-windows-gnu/easybooks.exe\n" if sidecar_style == "shasum" else win_hash + "\n"
    if corrupt == "mac":
        mac_text = ("0" * 64) + "  cli-aarch64-apple-darwin/easybooks\n"
    if omit != "mac":
        (mac_dir / "easybooks.sha256").write_text(mac_text)
    if omit != "win":
        (win_dir / "easybooks.exe.sha256").write_text(win_text)
    return mac_hash, win_hash


def _assemble(stage: Path, out: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["bash", str(ASSEMBLE), str(stage), str(out)],
        capture_output=True,
        text=True,
    )


class AssembleChecksumsTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp(prefix="easybooks-assemble-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)

    def test_copies_sealed_sidecars_and_stamps_manifest(self) -> None:
        stage = self.tmp / "stage"
        out = self.tmp / "plugin"
        mac_hash, win_hash = _write_stage(stage)
        proc = _assemble(stage, out)
        self.assertEqual(proc.returncode, 0, proc.stderr + proc.stdout)
        runtime = json.loads((out / "runtime-manifest.json").read_text())
        platforms = runtime["binary"]["platforms"]
        self.assertEqual(platforms["darwin-arm64"]["sha256"], mac_hash)
        self.assertEqual(platforms["win32-x64"]["sha256"], win_hash)
        # Preserve the sealed shasum text; do not rewrite to a new format.
        self.assertEqual(
            (out / "bin/darwin-arm64/easybooks.sha256").read_text(),
            f"{mac_hash}  cli-aarch64-apple-darwin/easybooks\n",
        )
        self.assertEqual(
            (out / "bin/win32-x64/easybooks.exe.sha256").read_text(),
            f"{win_hash}  cli-x86_64-pc-windows-gnu/easybooks.exe\n",
        )
        self.assertTrue(runtime.get("source_sha"))
        verify = subprocess.run(
            ["bash", str(out / "scripts" / "verify-package")],
            capture_output=True,
            text=True,
        )
        self.assertEqual(verify.returncode, 0, verify.stderr)

    def test_preflight_without_sidecars_still_assembles(self) -> None:
        stage = self.tmp / "stage"
        out = self.tmp / "plugin"
        _write_stage(stage, sidecars=False)
        proc = _assemble(stage, out)
        self.assertEqual(proc.returncode, 0, proc.stderr + proc.stdout)
        runtime = json.loads((out / "runtime-manifest.json").read_text())
        platforms = runtime["binary"]["platforms"]
        self.assertNotIn("sha256", platforms["darwin-arm64"])
        self.assertNotIn("sha256", platforms["win32-x64"])
        self.assertFalse((out / "bin/darwin-arm64/easybooks.sha256").exists())
        self.assertFalse((out / "bin/win32-x64/easybooks.exe.sha256").exists())

    def test_rejects_sidecar_that_does_not_match_binary(self) -> None:
        stage = self.tmp / "stage"
        out = self.tmp / "plugin"
        _write_stage(stage, corrupt="mac")
        proc = _assemble(stage, out)
        self.assertNotEqual(proc.returncode, 0)
        self.assertIn("does not match", proc.stderr)

    def test_rejects_partial_seal(self) -> None:
        stage = self.tmp / "stage"
        out = self.tmp / "plugin"
        _write_stage(stage, omit="win")
        proc = _assemble(stage, out)
        self.assertNotEqual(proc.returncode, 0)
        self.assertIn("missing", proc.stderr)


if __name__ == "__main__":
    unittest.main()
