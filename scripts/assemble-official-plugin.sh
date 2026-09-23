#!/usr/bin/env bash
# Assemble a complete EasyBooks public plugin tree from source + signed stage bins.
# Usage: assemble-official-plugin.sh <signed-stage-dir> <out-plugin-dir>
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
STAGE="${1:?signed stage dir}"
OUT="${2:?output plugin dir}"
rm -rf "$OUT"
mkdir -p "$OUT/bin/darwin-arm64" "$OUT/bin/win32-x64" "$OUT/skills" "$OUT/scripts"
cp -R "$ROOT/plugin-metadata/.claude-plugin" "$OUT/"
cp -R "$ROOT/plugin-metadata/.codex-plugin" "$OUT/"
cp "$ROOT/plugin-metadata/runtime-manifest.json" "$OUT/"
cp "$ROOT/plugin-metadata/README.md" "$OUT/"
cp -R "$ROOT/.claude/skills/easybooks" "$OUT/skills/easybooks"
for s in verify-package verify-release-assets verify-install; do
  cp "$ROOT/plugin-metadata/scripts/$s" "$OUT/scripts/$s"
  chmod +x "$OUT/scripts/$s"
done
cp "$STAGE/cli-aarch64-apple-darwin/easybooks" "$OUT/bin/darwin-arm64/easybooks"
cp "$STAGE/cli-x86_64-pc-windows-gnu/easybooks.exe" "$OUT/bin/win32-x64/easybooks.exe"
chmod +x "$OUT/bin/darwin-arm64/easybooks"
python3 - "$OUT" "$(git -C "$ROOT" rev-parse HEAD)" <<'PY'
import json, pathlib, sys
root = pathlib.Path(sys.argv[1])
sha = sys.argv[2]
rm = json.loads((root / "runtime-manifest.json").read_text())
rm["source_sha"] = sha
(root / "runtime-manifest.json").write_text(json.dumps(rm, indent=2) + "\n")
PY
# Post-sign finalize: hash the bins actually copied into the package (codesign
# changes Mach-O bytes). Writes *.sha256 sidecars + runtime-manifest sha256.
python3 "$ROOT/scripts/official-plugin/finalize-signed-hashes.py" --staged "$OUT"
echo "assembled $OUT"
