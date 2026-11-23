#!/usr/bin/env bash
# Verify the EasyBooks plugin tree is ready for first use after install/update.
# This is a packaging/readiness gate (CONTRACT §7), NOT a live backend test.
#
# Emits a single JSON object describing the result. Exit 0 = ready, 1 = not ready.
#
# Checks:
#   - manifests present: runtime-manifest.json + .claude-plugin/plugin.json (+ .codex-plugin/plugin.json if authored)
#   - skills present (.claude/skills) and non-empty
#   - the resolved CLI binary exists and is executable
#       resolution order (mirrors CONTRACT §0 binary resolver, local subset):
#         1. $EASYBOOKS_BIN
#         2. $CLAUDE_PLUGIN_ROOT/bin/<platform>/easybooks
#         3. <repo>/bin/<platform>/easybooks (local build via scripts/build-local.sh)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

failures=()

add_failure() {
  # $1 = code, $2 = detail
  failures+=("{\"code\":\"$1\",\"detail\":\"$2\"}")
}

require_file() {
  local path="$1"
  if [[ ! -f "$path" ]]; then
    add_failure "missing_file" "$path"
  fi
}

require_nonempty_dir() {
  local path="$1"
  if [[ ! -d "$path" ]]; then
    add_failure "missing_dir" "$path"
  elif [[ -z "$(ls -A "$path" 2>/dev/null)" ]]; then
    add_failure "empty_dir" "$path"
  fi
}

# --- Platform detection (for bin/<platform>/easybooks resolution) ---
detect_platform() {
  local os arch
  case "$(uname -s)" in
    Darwin) os="darwin" ;;
    Linux)  os="linux" ;;
    *)      os="unknown" ;;
  esac
  case "$(uname -m)" in
    arm64|aarch64) arch="arm64" ;;
    x86_64|amd64)  arch="x64" ;;
    *)             arch="unknown" ;;
  esac
  # Contract uses darwin-arm64 / darwin-x64 / linux-x64 / win32-x64.
  if [[ "$os" == "linux" ]]; then
    echo "linux-x64"
  else
    echo "${os}-${arch}"
  fi
}

PLATFORM="$(detect_platform)"

# --- Manifests ---
require_file "$ROOT/plugin-metadata/runtime-manifest.json"
require_file "$ROOT/plugin-metadata/.claude-plugin/plugin.json"
require_file "$ROOT/plugin-metadata/scripts/verify-install"
# .codex-plugin/plugin.json is part of the metadata superset; required if the dir exists.
if [[ -d "$ROOT/plugin-metadata/.codex-plugin" ]]; then
  require_file "$ROOT/plugin-metadata/.codex-plugin/plugin.json"
fi

# --- Skills present ---
require_nonempty_dir "$ROOT/.claude/skills"

# --- Resolved binary executable ---
resolved_bin=""
if [[ -n "${EASYBOOKS_BIN:-}" && -x "${EASYBOOKS_BIN}" ]]; then
  resolved_bin="${EASYBOOKS_BIN}"
elif [[ -n "${CLAUDE_PLUGIN_ROOT:-}" && -x "${CLAUDE_PLUGIN_ROOT}/bin/${PLATFORM}/easybooks" ]]; then
  resolved_bin="${CLAUDE_PLUGIN_ROOT}/bin/${PLATFORM}/easybooks"
elif [[ -x "${ROOT}/bin/${PLATFORM}/easybooks" ]]; then
  resolved_bin="${ROOT}/bin/${PLATFORM}/easybooks"
fi

if [[ -z "$resolved_bin" ]]; then
  add_failure "binary_unresolved" "no executable easybooks for platform ${PLATFORM} (set EASYBOOKS_BIN or run scripts/build-local.sh)"
elif [[ ! -x "$resolved_bin" ]]; then
  add_failure "not_executable" "$resolved_bin"
fi

# --- Emit JSON result ---
if [[ "${#failures[@]}" -gt 0 ]]; then
  joined="$(IFS=,; echo "${failures[*]}")"
  printf '{"ok":false,"checked":"runtime-readiness","platform":"%s","failures":[%s]}\n' "$PLATFORM" "$joined" >&2
  exit 1
fi

printf '{"ok":true,"checked":"runtime-readiness","platform":"%s","binary":"%s"}\n' "$PLATFORM" "$resolved_bin"
