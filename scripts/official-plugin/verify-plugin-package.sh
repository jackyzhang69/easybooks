#!/usr/bin/env bash
# Mechanical conformance gate for a public plugin package.
#
# Enforces the parts of delivery/plugin-policy.md and
# delivery/plugin-common-module.md §5 that a machine can check:
#   - Package shape conformance (manifest envelope, source_sha, bin naming)
#   - Any-product host OS floor (exactly darwin-arm64 + win32-x64)
#   - When bins are present and verifyChecksum is true: *.sha256 sidecars
#     plus runtime-manifest platform sha256 must match the binaries
#   - Version agreement across every metadata surface
#   - Unified runtime storage (configHome under ~/.jackyzhang.app/<plugin_id>/)
#   - Physical single SKILL.md (exactly one skills/*/SKILL.md; none under references/)
#   - Relative markdown links in the packed SKILL.md + references/ resolve
#   - Codex defaultPrompt / longDescription name the existing router directory
#   - Required verification scripts present
#   - Retired credential prefixes on every host surface
#
# Dated exemptions: scripts/plugin-package-exemptions.tsv
# (plugin_id, check, YYYY-MM-DD). Expired date = FAIL. No exemption for
# host-os-floor, missing commands --json, or multiple SKILL.md.
#
# Judgment calls stay in the policy documents. This only checks what is decidable.
#
# Usage: verify-plugin-package.sh <plugin-dir> [<plugin-dir>...]
#        verify-plugin-package.sh --marketplace <marketplace-repo>
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
EXEMPTIONS="$HERE/plugin-package-exemptions.tsv"
if [ ! -f "$EXEMPTIONS" ]; then
  EXEMPTIONS="$ROOT/scripts/plugin-package-exemptions.tsv"
fi
ROUTER_NAMES="anychat anypdf formbro easybooks anydoc anyimmi"

FAIL=0
CUR=""
PLUGIN_ID=""
note() { printf '    %s\n' "$*"; }
fail() {
  if exempt "$PLUGIN_ID" "$CUR"; then
    local until
    until="$(exemption_until "$PLUGIN_ID" "$CUR")"
    printf '  WARN  %-34s %s (exempt until %s)\n' "$PLUGIN_ID/$CUR" "$*" "$until"
    return
  fi
  printf '  FAIL  %-34s %s\n' "${PLUGIN_ID:-$CUR}" "$*"
  FAIL=1
}
warn() { printf '  WARN  %-34s %s\n' "${PLUGIN_ID:-$CUR}" "$*"; }
pass() { printf '  ok    %-34s %s\n' "${PLUGIN_ID:-$CUR}" "$*"; }

jqv() { python3 -c "
import json,sys
try: d=json.load(open(sys.argv[1]))
except Exception: sys.exit(2)
cur=d
for k in sys.argv[2].split('.'):
    if k=='' : continue
    if isinstance(cur,dict) and k in cur: cur=cur[k]
    else: sys.exit(1)
print(cur if not isinstance(cur,(dict,list)) else json.dumps(cur))
" "$1" "$2" 2>/dev/null; }

exemption_until() {
  local id="$1" check="$2"
  [ -f "$EXEMPTIONS" ] || return 1
  python3 - "$EXEMPTIONS" "$id" "$check" <<'PY'
import sys, datetime
path, plugin_id, check = sys.argv[1:4]
today = datetime.date.today()
for raw in open(path, encoding="utf-8"):
    line = raw.strip()
    if not line or line.startswith("#"):
        continue
    parts = line.split()
    if len(parts) != 3:
        continue
    pid, name, expires = parts
    if pid == plugin_id and name == check:
        print(expires)
        raise SystemExit(0)
raise SystemExit(1)
PY
}

exempt() {
  local id="$1" check="$2"
  [ -f "$EXEMPTIONS" ] || return 1
  python3 - "$EXEMPTIONS" "$id" "$check" <<'PY'
import sys, datetime
path, plugin_id, check = sys.argv[1:4]
today = datetime.date.today()
for raw in open(path, encoding="utf-8"):
    line = raw.strip()
    if not line or line.startswith("#"):
        continue
    parts = line.split()
    if len(parts) != 3:
        continue
    pid, name, expires = parts
    if pid != plugin_id or name != check:
        continue
    try:
        end = datetime.date.fromisoformat(expires)
    except ValueError:
        raise SystemExit(1)
    raise SystemExit(0 if today <= end else 1)
raise SystemExit(1)
PY
}

check_relative_markdown_links() {
  local dir="$1"
  python3 - "$dir" <<'PY'
import pathlib, re, sys
root = pathlib.Path(sys.argv[1])
skill_root = root / "skills"
files = []
for skill in skill_root.glob("*/SKILL.md"):
    files.append(skill)
    refs = skill.parent / "references"
    if refs.is_dir():
        files.extend(p for p in refs.rglob("*.md") if p.is_file())
link_re = re.compile(r"\[[^\]]*\]\(([^)]+)\)")
errors = []
for path in files:
    text = path.read_text(encoding="utf-8")
    for match in link_re.finditer(text):
        href = match.group(1).strip()
        if href.startswith(("<", "#", "http://", "https://", "mailto:")):
            continue
        href = href.split("#", 1)[0].split(" ", 1)[0].strip("<>")
        if not href or href.startswith("$"):
            continue
        target = (path.parent / href).resolve()
        try:
            target.relative_to(root.resolve())
        except ValueError:
            errors.append(f"{path.relative_to(root)}: link escapes package: {href}")
            continue
        if not target.exists():
            errors.append(f"{path.relative_to(root)}: broken relative link: {href}")
if errors:
    print("\n".join(errors))
    raise SystemExit(1)
PY
}

check_codex_router_name() {
  local xp="$1" router="$2"
  python3 - "$xp" "$router" <<'PY'
import json, sys
path, router = sys.argv[1:3]
data = json.load(open(path, encoding="utf-8"))
iface = data.get("interface") or {}
blob = " ".join([
    str(iface.get("longDescription") or ""),
    json.dumps(iface.get("defaultPrompt") or []),
])
if router not in blob:
    print(f"interface.defaultPrompt/longDescription do not name router {router!r}")
    raise SystemExit(1)
# Catch the last-round defect: stale *-capabilities name after a rename.
stale = f"{router}-capabilities"
if stale in blob and router not in (iface.get("longDescription") or ""):
    # If the stale token is present, the current router must still be named.
    pass
if stale in blob and router not in blob.replace(stale, ""):
    print(f"interface still names {stale} without the current router {router}")
    raise SystemExit(1)
PY
}

check_plugin() {
  local dir="$1"
  local id; id="$(basename "$dir")"
  PLUGIN_ID="$id"
  printf '\n== %s\n' "$id"

  local rm="$dir/runtime-manifest.json"
  local cp="$dir/.claude-plugin/plugin.json"
  local xp="$dir/.codex-plugin/plugin.json"

  # --- host metadata surfaces -------------------------------------------------
  CUR="host-metadata"
  [ -f "$cp" ] || fail "missing .claude-plugin/plugin.json"
  [ -f "$xp" ] || warn "no .codex-plugin/plugin.json (plugin absent from Codex host)"
  [ -f "$rm" ] || { fail "missing runtime-manifest.json"; return; }
  [ -f "$cp" ] && [ -f "$rm" ] && pass "metadata present"

  # A hand-maintained root manifest.json is not a supported surface.
  CUR="root-manifest"
  if [ -f "$dir/manifest.json" ]; then
    fail "root manifest.json is not a supported metadata surface (duplicates host plugin.json)"
  else
    pass "no unsupported root manifest"
  fi

  # --- version agreement ------------------------------------------------------
  CUR="version-agreement"
  local vc vx
  vc="$(jqv "$cp" version)"
  vx="$(jqv "$xp" version)"
  local ref="$vc"
  if [ -z "$ref" ]; then
    warn "no version found to compare"
  else
    local bad=0
    if [ -n "$vx" ] && [ "$vx" != "$ref" ]; then fail "codex=$vx disagrees with $ref"; bad=1; fi
    local runtime_mismatch
    runtime_mismatch="$(python3 - "$rm" "$ref" <<'PY'
import json, re, sys
path, ref = sys.argv[1], sys.argv[2]
product = re.compile(r"^\d+\.\d+")

def envelope(value):
    return isinstance(value, int) or (isinstance(value, str) and value.isdigit())

rm = json.load(open(path, encoding="utf-8"))
pairs = []
raw = rm.get("version")
if raw is not None and not envelope(raw) and product.match(str(raw)):
    pairs.append(("runtime.version", str(raw)))
binary = rm.get("binary")
if isinstance(binary, dict):
    bv = binary.get("version")
    if bv is not None and not envelope(bv) and product.match(str(bv)):
        pairs.append(("runtime.binary.version", str(bv)))
pv = rm.get("plugin_version")
if pv is not None and not envelope(pv) and product.match(str(pv)):
    pairs.append(("runtime.plugin_version", str(pv)))
bad = [f"{name}={value}" for name, value in pairs if value != ref]
print("\n".join(bad))
raise SystemExit(1 if bad else 0)
PY
)" || true
    if [ -n "$runtime_mismatch" ]; then
      while IFS= read -r line; do
        [ -n "$line" ] && fail "$line disagrees with $ref"
      done <<< "$runtime_mismatch"
      bad=1
    fi
    [ $bad -eq 0 ] && pass "all metadata report $ref"
  fi

  # --- source_sha -------------------------------------------------------------
  CUR="source-sha"
  if [ -n "$(jqv "$rm" source_sha)" ]; then pass "source_sha recorded"
  else fail "runtime-manifest.json has no source_sha (release binds one exact commit)"; fi

  # --- host OS floor + bin naming --------------------------------------------
  CUR="host-os-floor"
  local have_mac=0 have_win=0 extra=""
  for d in "$dir"/bin/*/; do
    [ -d "$d" ] || continue
    case "$(basename "$d")" in
      darwin-arm64) have_mac=1 ;;
      win32-x64)    have_win=1 ;;
      *)            extra="$extra $(basename "$d")" ;;
    esac
  done
  [ $have_mac -eq 1 ] || fail "no bin/darwin-arm64/"
  [ $have_win -eq 1 ] || fail "no bin/win32-x64/ (macOS+Windows is a release floor, not a roadmap item)"
  [ -n "$extra" ] && fail "unexpected binary dirs:$extra (public packages ship exactly two)"
  [ $have_mac -eq 1 ] && [ $have_win -eq 1 ] && [ -z "$extra" ] && pass "exactly darwin-arm64 + win32-x64"

  # --- checksum sidecars when verifyChecksum is on and bins are present --------
  CUR="checksum-sidecars"
  if [ $have_mac -eq 1 ] || [ $have_win -eq 1 ]; then
    local checksum_msg
    if checksum_msg="$(python3 - "$rm" "$dir" <<'PY'
import hashlib, json, pathlib, sys
manifest_path, root = sys.argv[1], pathlib.Path(sys.argv[2])
runtime = json.loads(pathlib.Path(manifest_path).read_text(encoding="utf-8"))
binary = runtime.get("binary") or {}
if binary.get("verifyChecksum") is not True:
    raise SystemExit(0)
errors = []
for plat, entry in (binary.get("platforms") or {}).items():
    if not isinstance(entry, dict):
        errors.append(f"binary.{plat}: platform entry is not an object")
        continue
    rel = entry.get("entrypoint")
    digest = entry.get("sha256")
    if not rel or not digest:
        errors.append(f"binary.{plat}: missing entrypoint or sha256")
        continue
    artifact = root / rel
    if artifact.is_symlink() or not artifact.is_file():
        errors.append(f"binary.{plat}: missing artifact {rel}")
        continue
    actual = hashlib.sha256(artifact.read_bytes()).hexdigest()
    if actual != digest:
        errors.append(f"binary.{plat}: sha256 drift manifest={digest} actual={actual}")
    sidecar = artifact.with_name(artifact.name + ".sha256")
    if sidecar.is_symlink() or not sidecar.is_file():
        errors.append(f"binary.{plat}: missing sidecar {sidecar.name}")
    else:
        side = sidecar.read_text(encoding="utf-8").strip().split()[0]
        if side != digest:
            errors.append(f"binary.{plat}: sidecar {side} != manifest {digest}")
if errors:
    print("\n".join(errors))
    raise SystemExit(1)
PY
)"; then
      pass "sidecars and runtime-manifest sha256 agree"
    else
      if [ -n "$checksum_msg" ]; then
        while IFS= read -r line; do
          [ -n "$line" ] && fail "$line"
        done <<< "$checksum_msg"
      else
        fail "checksum sidecars do not match runtime-manifest sha256"
      fi
    fi
  else
    pass "no bins to checksum (metadata-only tree)"
  fi

  # --- unified runtime storage ------------------------------------------------
  CUR="runtime-storage"
  local ch; ch="$(jqv "$rm" configHome)"
  if [ -z "$ch" ]; then warn "no configHome declared"
  elif printf '%s' "$ch" | grep -q 'jackyzhang.app'; then pass "configHome under ~/.jackyzhang.app"
  else fail "configHome outside ~/.jackyzhang.app/<plugin_id>/: $ch"; fi

  # --- physical single SKILL.md ----------------------------------------------
  CUR="single-entry"
  local skill_files=()
  local skill
  while IFS= read -r skill; do
    [ -n "$skill" ] && skill_files+=("$skill")
  done < <(find "$dir/skills" -name SKILL.md -print 2>/dev/null | sort)
  local count="${#skill_files[@]}"
  if [ "$count" -ne 1 ]; then
    fail "$count skills/*/SKILL.md (expected exactly 1): ${skill_files[*]#$dir/}"
  else
    local rel="${skill_files[0]#$dir/}"
    local router
    router="$(basename "$(dirname "${skill_files[0]}")")"
    case " $ROUTER_NAMES " in
      *" $router "*) pass "exactly one SKILL.md ($rel)" ;;
      *) fail "router directory $router is not a product router ($ROUTER_NAMES)" ;;
    esac
    CUR="references-skill"
    if find "$dir/skills" -path '*/references/*/SKILL.md' -print -quit 2>/dev/null | grep -q .; then
      fail "SKILL.md found under references/"
    else
      pass "no SKILL.md under references/"
    fi
    CUR="markdown-links"
    if check_relative_markdown_links "$dir"; then
      pass "relative markdown links resolve inside the package"
    else
      fail "broken or escaping relative markdown links"
    fi
    CUR="codex-router"
    if [ -f "$xp" ]; then
      local codex_msg
      if codex_msg="$(check_codex_router_name "$xp" "$router")"; then
        pass "Codex interface names router $router"
      else
        fail "${codex_msg:-Codex interface does not name router $router}"
      fi
    fi
  fi

  # --- required verification scripts ------------------------------------------
  CUR="verify-scripts"
  local missing=""
  for s in verify-package verify-release-assets verify-install; do
    [ -e "$dir/scripts/$s" ] || missing="$missing $s"
  done
  if [ -z "$missing" ]; then pass "all verification scripts present"
  else fail "missing scripts:$missing"; fi

  # --- retired credential prefixes on every host surface ----------------------
  CUR="retired-credentials"
  local hits
  hits="$(grep -rlE 'eb_live_|ap_live_|fb_[a-z]' "$dir" 2>/dev/null \
          | xargs -I{} grep -LiE 'retired|rejected' {} 2>/dev/null || true)"
  if [ -n "$hits" ]; then
    fail "retired credential prefix taught to users:"
    printf '%s\n' "$hits" | sed 's/^/          /'
  else
    pass "no retired credential instructions"
  fi
}

main() {
  local dirs=()
  if [ "${1:-}" = "--marketplace" ]; then
    local mk="${2:?--marketplace needs a repo path}"
    # Catalog vs tree divergence: a directory nobody can install is a defect.
    CUR="catalog"
    PLUGIN_ID="marketplace"
    printf '== marketplace %s\n' "$mk"
    local cat="$mk/.claude-plugin/marketplace.json"
    if [ -f "$cat" ]; then
      local listed; listed="$(python3 -c "
import json;print(' '.join(p['name'] for p in json.load(open('$cat'))['plugins']))")"
      for d in "$mk"/plugins/*/; do
        local n; n="$(basename "$d")"
        case " $listed " in *" $n "*) ;; *) fail "plugins/$n/ is in the tree but not in marketplace.json";; esac
      done
      [ $FAIL -eq 0 ] && pass "catalog matches tree"
    else
      fail "no .claude-plugin/marketplace.json"
    fi
    for d in "$mk"/plugins/*/; do dirs+=("${d%/}"); done
  else
    for a in "$@"; do dirs+=("${a%/}"); done
  fi
  [ ${#dirs[@]} -gt 0 ] || { echo "usage: $0 <plugin-dir>... | --marketplace <repo>"; exit 2; }
  for d in "${dirs[@]}"; do check_plugin "$d"; done
  printf '\n'
  if [ $FAIL -ne 0 ]; then echo "RESULT: FAIL"; exit 1; fi
  echo "RESULT: PASS"
}
main "$@"
