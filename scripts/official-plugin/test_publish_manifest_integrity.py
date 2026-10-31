"""Publish-path gate: runtime-manifest hashes must match the shipped bytes.

Regression for AnyScore 0.2.2/0.2.3, which shipped a darwin-arm64 sha256 and
size taken by stage-package before codesign/notarize rewrote the binary.
"""
from __future__ import annotations

import hashlib
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "publish-official-plugin.py"
spec = importlib.util.spec_from_file_location("publish_official_plugin_integrity", SCRIPT)
assert spec and spec.loader
pub = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pub)


def write_artifact(root: Path, rel: str, data: bytes) -> dict:
    path = root / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)
    digest = hashlib.sha256(data).hexdigest()
    path.with_name(path.name + ".sha256").write_text(digest + "\n")
    return {"entrypoint": rel, "sha256": digest}


class ManifestIntegrityGate(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)

    def tearDown(self) -> None:
        self._tmp.cleanup()

    def write_manifest(self, manifest: dict) -> None:
        (self.root / "runtime-manifest.json").write_text(json.dumps(manifest))

    def flat_package(self) -> None:
        platforms = {}
        for name, rel in (
            ("darwin-arm64", "bin/darwin-arm64/tool"),
            ("win32-x64", "bin/win32-x64/tool.exe"),
            ("linux-x64", "bin/linux-x64/tool"),
        ):
            entry = write_artifact(self.root, rel, f"unsigned {name}".encode())
            entry["size"] = len(f"unsigned {name}")
            platforms[name] = entry
        self.write_manifest({"version": "1.0.0", "platforms": platforms})

    def test_consistent_package_passes(self) -> None:
        self.flat_package()
        self.assertEqual(pub.manifest_integrity_errors(self.root), [])
        pub.require_manifest_integrity(self.root, "staged package")

    def test_signing_after_staging_fails_closed(self) -> None:
        self.flat_package()
        darwin = self.root / "bin/darwin-arm64/tool"
        darwin.write_bytes(darwin.read_bytes() + b"codesign")
        errors = pub.manifest_integrity_errors(self.root)
        self.assertTrue(any("darwin-arm64: sha256" in e for e in errors), errors)
        self.assertTrue(any("darwin-arm64: size" in e for e in errors), errors)
        self.assertTrue(any(e.startswith("bin/darwin-arm64/tool.sha256") for e in errors), errors)
        self.assertFalse(any("win32-x64" in e or "linux-x64" in e for e in errors), errors)
        with self.assertRaises(pub.PublishError):
            pub.require_manifest_integrity(self.root, "staged package")

    def test_nested_binary_helper_schema_and_size_bytes(self) -> None:
        binary = write_artifact(self.root, "bin/darwin-arm64/tool", b"tool")
        helper = write_artifact(self.root, "bin/darwin-arm64/tool-access", b"helper")
        helper["size_bytes"] = 999
        lib = write_artifact(self.root, "bin/darwin-arm64/libpdfium.dylib", b"lib")
        lib = {"path": lib["entrypoint"], "sha256": lib["sha256"]}
        binary["pdfium"] = lib
        self.write_manifest({
            "binary": {"platforms": {"darwin-arm64": binary}},
            "helper": {"platforms": {"darwin-arm64": helper}},
        })
        errors = pub.manifest_integrity_errors(self.root)
        self.assertEqual(len(errors), 1, errors)
        self.assertIn("helper.platforms.darwin-arm64: size_bytes 999", errors[0])
        (self.root / "bin/darwin-arm64/libpdfium.dylib").write_bytes(b"resigned")
        errors = pub.manifest_integrity_errors(self.root)
        self.assertTrue(any("pdfium: sha256" in e for e in errors), errors)

    def test_recorded_but_unshipped_fails_and_release_hosted_is_skipped(self) -> None:
        self.write_manifest({
            "platforms": {"linux-x64": {"entrypoint": "bin/linux-x64/tool", "sha256": "0" * 64}},
            "runtime": {"platforms": {"linux-x64": {
                "entrypoint": "runtime/linux-x64/rt", "asset": "rt-linux-x64", "sha256": "0" * 64,
            }}},
        })
        errors = pub.manifest_integrity_errors(self.root)
        self.assertEqual(len(errors), 1, errors)
        self.assertIn("bin/linux-x64/tool is recorded but not shipped", errors[0])


if __name__ == "__main__":
    unittest.main()
