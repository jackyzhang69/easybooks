#!/usr/bin/env python3
"""Write post-sign .sha256 sidecars and runtime-manifest platform sha256 fields.

AnyChat-style finalize step. Signing/notarization changes Mach-O bytes, so this
must run against the staged plugin tree *after* `sign-macos-cli.sh`. Source
`plugin-metadata/runtime-manifest.json` stays hash-free; only a staged package
with real bins is finalized.

Sidecar format matches AnyChat: a single hex digest plus newline, named
`<binary>.sha256` next to the entrypoint. `verifyChecksum: true` packages must
have both the sidecar and `binary.platforms.<plat>.sha256`.

Usage:
  python3 finalize-signed-hashes.py --staged DIR
  python3 finalize-signed-hashes.py --staged DIR --check
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path
from typing import Any


class FinalizeError(SystemExit):
    def __init__(self, message: str, code: int = 2) -> None:
        super().__init__(code)
        self.message = message
        print(f"finalize-signed-hashes: {message}", file=sys.stderr)


def _sha256_file(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def _sidecar_path(artifact: Path) -> Path:
    return artifact.with_name(artifact.name + ".sha256")


def _platform_sections(runtime: dict[str, Any]) -> list[tuple[str, dict[str, Any]]]:
    found: list[tuple[str, dict[str, Any]]] = []
    for name in ("binary", "helper"):
        section = runtime.get(name)
        if isinstance(section, dict) and isinstance(section.get("platforms"), dict):
            found.append((name, section))
    return found


def _iter_platform_entries(
    runtime: dict[str, Any],
) -> list[tuple[str, str, dict[str, Any]]]:
    entries: list[tuple[str, str, dict[str, Any]]] = []
    for comp, section in _platform_sections(runtime):
        platforms = section.get("platforms") or {}
        for plat, entry in platforms.items():
            if not isinstance(entry, dict):
                raise FinalizeError(f"{comp}.{plat}: platform entry is not an object")
            entries.append((comp, str(plat), entry))
    if not entries:
        raise FinalizeError("runtime-manifest.json has no binary/helper platforms")
    return entries


def _require_artifact(staged: Path, rel: str, label: str) -> Path:
    if not rel:
        raise FinalizeError(f"{label}: missing entrypoint")
    artifact = staged / rel
    if artifact.is_symlink() or not artifact.is_file():
        raise FinalizeError(f"{label}: staged artifact missing or unsafe: {rel}")
    return artifact


def _sidecar_digest(sidecar: Path) -> str:
    text = sidecar.read_text(encoding="utf-8").strip()
    if not text:
        raise FinalizeError(f"empty checksum sidecar: {sidecar.name}")
    return text.split()[0]


def require_signed_hashes(staged: Path) -> dict[str, Any]:
    """Read-only gate: sidecars + manifest sha256 must match the binaries."""
    staged = staged.resolve()
    path = staged / "runtime-manifest.json"
    if not path.is_file():
        raise FinalizeError("staged package missing runtime-manifest.json")
    runtime = json.loads(path.read_text(encoding="utf-8"))
    recorded: dict[str, str] = {}
    for comp, plat, entry in _iter_platform_entries(runtime):
        label = f"{comp}.{plat}"
        rel = str(entry.get("entrypoint") or "")
        digest = entry.get("sha256")
        if not isinstance(digest, str) or not digest:
            raise FinalizeError(f"{label}: missing sha256 (run finalize-signed-hashes.py)")
        artifact = _require_artifact(staged, rel, label)
        actual = _sha256_file(artifact)
        if actual != digest:
            raise FinalizeError(
                f"{label}: sha256 drift for {rel}: manifest={digest} actual={actual}"
            )
        sidecar = _sidecar_path(artifact)
        if sidecar.is_symlink() or not sidecar.is_file():
            raise FinalizeError(f"{label}: sidecar missing or unsafe: {sidecar.name}")
        side = _sidecar_digest(sidecar)
        if side != digest:
            raise FinalizeError(f"{label}: sidecar {sidecar.name}={side} != manifest {digest}")
        recorded[f"{comp}.{plat}"] = digest
    return {"ok": True, "mode": "check", "staged": str(staged), "sha256": recorded}


def finalize_signed_hashes(staged: Path) -> dict[str, Any]:
    """Hash staged bins, write `.sha256` sidecars, and stamp the manifest."""
    staged = staged.resolve()
    path = staged / "runtime-manifest.json"
    if not path.is_file():
        raise FinalizeError("staged package missing runtime-manifest.json")
    runtime = json.loads(path.read_text(encoding="utf-8"))
    recorded: dict[str, str] = {}
    for comp, plat, entry in _iter_platform_entries(runtime):
        label = f"{comp}.{plat}"
        rel = str(entry.get("entrypoint") or "")
        artifact = _require_artifact(staged, rel, label)
        digest = _sha256_file(artifact)
        sidecar = _sidecar_path(artifact)
        sidecar.write_text(digest + "\n", encoding="utf-8")
        entry["sha256"] = digest
        recorded[f"{comp}.{plat}"] = digest
    path.write_text(json.dumps(runtime, indent=2) + "\n", encoding="utf-8")
    return {"ok": True, "mode": "write", "staged": str(staged), "sha256": recorded}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--staged", required=True, type=Path)
    parser.add_argument(
        "--check",
        action="store_true",
        help="verify sidecars and manifest sha256 without writing",
    )
    args = parser.parse_args(argv)
    try:
        result = (
            require_signed_hashes(args.staged)
            if args.check
            else finalize_signed_hashes(args.staged)
        )
    except FinalizeError:
        return 2
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
