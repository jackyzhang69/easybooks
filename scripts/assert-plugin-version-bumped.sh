#!/usr/bin/env bash
# Fail closed: plugin sources changed since last plugin-v* tag => workspace version must increase.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
python3 - <<'PY'
from __future__ import annotations
import json, re, subprocess, sys
from pathlib import Path

def ver_tuple(s: str) -> tuple[int, int, int]:
    parts = s.split(".")
    if len(parts) != 3 or not all(p.isdigit() for p in parts):
        raise SystemExit(json.dumps({"ok": False, "code": "version_invalid", "version": s}))
    return int(parts[0]), int(parts[1]), int(parts[2])

cargo = Path("Cargo.toml").read_text(encoding="utf-8")
# workspace.package version is first version = under [workspace.package]
m = re.search(r'\[workspace\.package\]\s*\n(?:[^\n]*\n)*?version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"', cargo)
if not m:
    # fallback first version in file
    m = re.search(r'version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"', cargo)
assert m, "workspace version missing"
client = m.group(1)

for jp in Path("plugin-metadata").rglob("plugin.json"):
    data = json.loads(jp.read_text(encoding="utf-8"))
    if data.get("version") != client:
        print(json.dumps({"ok": False, "code": "plugin_json_version_mismatch", "path": str(jp), "version": data.get("version"), "cargo": client}, sort_keys=True))
        raise SystemExit(2)

tags = subprocess.check_output(["git", "tag", "-l", "plugin-v*", "--sort=-v:refname"], text=True).splitlines()
last_tag = tags[0] if tags else ""
if not last_tag:
    print(json.dumps({"ok": True, "detail": "no prior plugin-v tag", "version": client}, sort_keys=True))
    raise SystemExit(0)
last_ver = last_tag.removeprefix("plugin-v")
changed = subprocess.check_output(
    ["git", "diff", "--name-only", f"{last_tag}..HEAD", "--",
     "cli/", "plugin-metadata/", "Cargo.toml", "Cargo.lock", ".claude/skills/"],
    text=True,
).strip()
if not changed:
    print(json.dumps({"ok": True, "detail": f"no plugin paths changed since {last_tag}", "version": client}, sort_keys=True))
    raise SystemExit(0)
if ver_tuple(client) <= ver_tuple(last_ver):
    print(json.dumps({
        "ok": False,
        "code": "plugin_version_not_bumped",
        "version": client,
        "last_release": last_tag,
        "changed_paths": changed.splitlines()[:40],
        "detail": "plugin paths changed since last plugin-v tag but version was not increased",
    }, sort_keys=True))
    raise SystemExit(2)
print(json.dumps({"ok": True, "version": client, "last_release": last_tag}, sort_keys=True))
PY
