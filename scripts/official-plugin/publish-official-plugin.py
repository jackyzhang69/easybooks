#!/usr/bin/env python3
"""Fail-closed official plugin publisher.

One run publishes one plugin only when the staged package carries the same
version and source commit on all three surfaces:

  * darwin-arm64 + win32-x64 CLI binaries
  * public plugin tree (exactly one skills/<product>/SKILL.md)
  * live production accountd allowlist (exchange audience) or FormBro catalog

It is the only supported writer of jackyzhang69/plugins. Skills-only copies
and catalog/CLI version skew are failed releases.

Usage:
  python3 publish-official-plugin.py --plugin-id anychat --staged DIR \\
      --marketplace DIR --apply
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any


HERE = Path(__file__).resolve().parent
VAULT = HERE.parent
EXCHANGE_URL = os.environ.get(
    "OFFICIAL_PLUGIN_EXCHANGE_URL",
    "https://account.jackyzhang.app/v1/token/exchange",
)
CATALOG_URL = os.environ.get(
    "OFFICIAL_PLUGIN_CATALOG_URL",
    "https://account.jackyzhang.app/v1/catalog/products",
)

# plugin_id -> marketplace catalog name, CLI bin, exchange aud (None = FormBro introspect).
# Catalog install name equals plugin_id. The `-cli` suffix is forbidden.
PLUGINS: dict[str, dict[str, str | None]] = {
    "anychat": {"catalog": "anychat", "bin": "anychat", "aud": "anychat"},
    "easybooks": {"catalog": "easybooks", "bin": "easybooks", "aud": "eb"},
    "formbro": {"catalog": "formbro", "bin": "formbro", "aud": None},
    "anypdf": {"catalog": "anypdf", "bin": "anypdf", "aud": "anypdf"},
    "anydoc": {"catalog": "anydoc", "bin": "anydoc", "aud": "anydoc"},
    "anyimmi": {"catalog": "anyimmi", "bin": "anyimmi", "aud": "anyimmi"},
}


def leftover_cli_catalog(catalog: str) -> str:
    """Pre-cutover marketplace name. Not an install alias."""
    return f"{catalog}-cli"


class PublishError(SystemExit):
    def __init__(self, message: str, code: int = 2) -> None:
        super().__init__(code)
        self.message = message
        print(f"publish-official-plugin: {message}", file=sys.stderr)


def _json_load(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def _tree_digest(root: Path) -> str:
    h = hashlib.sha256()
    if not root.exists():
        return h.hexdigest()
    for path in sorted(root.rglob("*")):
        rel = path.relative_to(root).as_posix()
        h.update(rel.encode())
        if path.is_file() and not path.is_symlink():
            h.update(path.read_bytes())
    return h.hexdigest()


def catalog_files(marketplace: Path) -> list[Path]:
    return [
        marketplace / ".claude-plugin" / "marketplace.json",
        marketplace / ".agents" / "plugins" / "marketplace.json",
    ]


def catalog_version(marketplace: Path, catalog: str) -> str | None:
    for path in catalog_files(marketplace):
        if not path.is_file():
            continue
        data = _json_load(path)
        for plugin in data.get("plugins") or []:
            if plugin.get("name") == catalog:
                return str(plugin.get("version") or "")
    return None



def resolve_render_public_plugin_docs() -> Path:
    env = os.environ.get("RENDER_PUBLIC_PLUGIN_DOCS")
    if env:
        path = Path(env)
        if path.is_file():
            return path
        raise PublishError(f"RENDER_PUBLIC_PLUGIN_DOCS is not a file: {env}")
    candidates = [
        Path("/Users/jacky/platform/governance/scripts/official-plugin/render_public_plugin_docs.py"),
        Path.home()
        / ".local/share/platform-governance/governance/scripts/official-plugin/render_public_plugin_docs.py",
        Path("/agent/repos/platform/governance/scripts/official-plugin/render_public_plugin_docs.py"),
    ]
    for path in candidates:
        if path.is_file():
            return path
    raise PublishError(
        "render_public_plugin_docs.py not found (set RENDER_PUBLIC_PLUGIN_DOCS or install platform governance)"
    )


def run_render_public_plugin_docs(args: list[str]) -> None:
    script = resolve_render_public_plugin_docs()
    proc = subprocess.run(
        [sys.executable, str(script), *args],
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        detail = (proc.stderr or proc.stdout or "").strip()
        raise PublishError(f"render_public_plugin_docs failed: {detail}")


def rewrite_marketplace_readme(marketplace: Path) -> None:
    run_render_public_plugin_docs(
        ["marketplace", "--marketplace", str(marketplace), "--write"]
    )


def rewrite_package_readme(staged: Path) -> None:
    run_render_public_plugin_docs(["package", "--staged", str(staged), "--write"])


def update_marketplace_readme(marketplace: Path, catalog: str, version: str) -> None:
    """Full-table regen; catalog/version already live in marketplace.json."""
    del catalog, version
    rewrite_marketplace_readme(marketplace)


def set_catalog_version(marketplace: Path, catalog: str, version: str) -> None:
    leftover = leftover_cli_catalog(catalog)
    source = f"./plugins/{catalog}"
    for path in catalog_files(marketplace):
        if not path.is_file():
            continue
        data = _json_load(path)
        found = False
        kept: list[Any] = []
        for plugin in data.get("plugins") or []:
            name = plugin.get("name")
            if name == catalog:
                if found:
                    continue
                plugin["version"] = version
                plugin["source"] = source
                kept.append(plugin)
                found = True
            elif name == leftover:
                if found:
                    continue
                plugin["name"] = catalog
                plugin["version"] = version
                plugin["source"] = source
                kept.append(plugin)
                found = True
            else:
                kept.append(plugin)
        if not found:
            raise PublishError(f"{path}: catalog has no entry {catalog} or {leftover}")
        data["plugins"] = kept
        path.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def staged_cli_version(text: str) -> str:
    text = text.strip()
    if text.startswith("{"):
        data = json.loads(text)
        version = data.get("version")
        if not version:
            raise PublishError("CLI JSON --version has no version field")
        return str(version)
    tokens = text.replace("=", " ").split()
    if not tokens:
        raise PublishError("CLI --version produced empty output")
    return tokens[-1]


def run_cli(bin_path: Path, args: list[str]) -> subprocess.CompletedProcess[str]:
    if bin_path.suffix == ".py":
        cmd = [sys.executable, str(bin_path), *args]
    else:
        cmd = [str(bin_path), *args]
    return subprocess.run(cmd, capture_output=True, text=True)


def darwin_bin(staged: Path, bin_name: str) -> Path:
    mac = staged / "bin" / "darwin-arm64"
    for candidate in (mac / bin_name, mac / f"{bin_name}.py"):
        if candidate.exists():
            return candidate
    raise PublishError(f"missing staged darwin binary {bin_name} under {mac}")


def require_dual_bins(staged: Path) -> None:
    if not (staged / "bin" / "darwin-arm64").is_dir():
        raise PublishError("staged package missing bin/darwin-arm64")
    if not (staged / "bin" / "win32-x64").is_dir():
        raise PublishError("staged package missing bin/win32-x64")


def require_marketplace_key(marketplace: Path | None = None) -> None:
    if os.environ.get("OFFICIAL_PLUGIN_SKIP_MARKETPLACE_KEY") == "1":
        return
    key = os.environ.get("PLUGINS_REPO_DEPLOY_KEY", "").strip()
    if key:
        return
    # Tagged CI must never succeed on an empty secret even if the clone origin
    # already points at jackyzhang69/plugins (ssh-agent is what actually pushes).
    if os.environ.get("GITHUB_ACTIONS") == "true":
        raise PublishError(
            "PLUGINS_REPO_DEPLOY_KEY is empty; refusing to publish from GitHub Actions"
        )
    if marketplace is not None:
        proc = subprocess.run(
            ["git", "-C", str(marketplace), "remote", "get-url", "origin"],
            capture_output=True,
            text=True,
        )
        if proc.returncode == 0 and "jackyzhang69/plugins" in proc.stdout:
            return
    raise PublishError(
        "PLUGINS_REPO_DEPLOY_KEY is empty and marketplace origin is not "
        "jackyzhang69/plugins; refusing to publish"
    )


def probe_exchange_audience(aud: str) -> str:
    override = os.environ.get("OFFICIAL_PLUGIN_ALLOWLIST", "").strip()
    if override:
        apps = {item.strip() for item in override.split(",") if item.strip()}
        return "invalid_token" if aud in apps else "unknown_audience"
    body = json.dumps({"aud": aud, "scopes": ["read", "write"]}).encode()
    req = urllib.request.Request(
        EXCHANGE_URL,
        data=body,
        method="POST",
        headers={
            "Authorization": "Bearer jz_probe_not_a_real_token",
            "Content-Type": "application/json",
            "User-Agent": "official-plugin-publish/1",
            "Accept": "application/json",
        },
    )
    try:
        with urllib.request.urlopen(req, timeout=15) as resp:
            raw = resp.read().decode()
    except urllib.error.HTTPError as exc:
        raw = exc.read().decode() or "{}"
    except urllib.error.URLError as exc:
        raise PublishError(f"live accountd exchange probe failed: {exc}") from exc
    try:
        payload = json.loads(raw or "{}")
    except json.JSONDecodeError as exc:
        raise PublishError(f"live exchange probe returned non-JSON: {raw[:180]!r}") from exc
    err = payload.get("error") or {}
    return str(err.get("code") or "")


def probe_formbro_catalog() -> None:
    override = os.environ.get("OFFICIAL_PLUGIN_CATALOG_IDS", "").strip()
    if override:
        ids = {item.strip() for item in override.split(",") if item.strip()}
        if "formbro" not in ids:
            raise PublishError("live catalog does not list formbro")
        return
    req = urllib.request.Request(
        CATALOG_URL,
        method="GET",
        headers={"User-Agent": "official-plugin-publish/1", "Accept": "application/json"},
    )
    try:
        with urllib.request.urlopen(req, timeout=15) as resp:
            payload = json.loads(resp.read().decode())
    except urllib.error.URLError as exc:
        raise PublishError(f"live catalog probe failed: {exc}") from exc
    products = payload.get("products") or []
    if not any(p.get("id") == "formbro" and p.get("enabled") for p in products):
        raise PublishError("live catalog does not list enabled formbro")


def probe_backend(plugin_id: str, aud: str | None) -> None:
    if aud is None:
        probe_formbro_catalog()
        return
    code = probe_exchange_audience(aud)
    if code == "unknown_audience":
        raise PublishError(
            f"live ACCOUNTD_APPS does not register audience {aud} for {plugin_id}"
        )
    if code != "invalid_token":
        raise PublishError(
            f"live exchange probe for aud={aud} returned {code!r}; expected invalid_token"
        )


def run_finalize_signed_hashes_check(staged: Path) -> None:
    script = HERE / "finalize-signed-hashes.py"
    if not script.is_file():
        raise PublishError(f"missing {script}")
    proc = subprocess.run(
        [sys.executable, str(script), "--staged", str(staged), "--check"],
        capture_output=True,
        text=True,
    )
    sys.stderr.write(proc.stdout)
    sys.stderr.write(proc.stderr)
    if proc.returncode != 0:
        raise PublishError(
            "finalize-signed-hashes.py --check failed "
            "(need .sha256 sidecars and runtime-manifest platform sha256 after signing)"
        )


def run_verify_package(staged: Path) -> None:
    script = HERE / "verify-plugin-package.sh"
    if not script.is_file():
        raise PublishError(f"missing {script}")
    proc = subprocess.run(["bash", str(script), str(staged)], capture_output=True, text=True)
    sys.stderr.write(proc.stdout)
    sys.stderr.write(proc.stderr)
    if proc.returncode != 0:
        raise PublishError("verify-plugin-package.sh failed on staged package")


def run_skill_surface(staged: Path, plugin_id: str, bin_path: Path) -> None:
    script = HERE / "assert-skill-surface.py"
    if not script.is_file():
        raise PublishError(f"missing {script}")
    skills = staged / "skills" / plugin_id
    proc = subprocess.run(
        [sys.executable, str(script), "--bin", str(bin_path), "--skills", str(skills)],
        capture_output=True,
        text=True,
    )
    sys.stderr.write(proc.stdout)
    sys.stderr.write(proc.stderr)
    if proc.returncode != 0:
        raise PublishError("assert-skill-surface.py failed against the staged binary")


def commands_json(bin_path: Path) -> dict[str, Any]:
    proc = run_cli(bin_path, ["commands", "--json"])
    if proc.returncode != 0:
        raise PublishError(
            "staged binary has no commands --json (skills-only marketplace write is a failed release)"
        )
    try:
        payload = json.loads(proc.stdout)
    except json.JSONDecodeError as exc:
        raise PublishError("commands --json is not JSON") from exc
    if payload.get("schema") != "plugin-cli-commands-v1":
        raise PublishError("commands --json schema is not plugin-cli-commands-v1")
    return payload


PRODUCT_VERSION = re.compile(r"^\d+\.\d+")


def is_envelope_version(value: Any) -> bool:
    if isinstance(value, bool):
        return True
    if isinstance(value, int):
        return True
    if isinstance(value, str) and value.isdigit():
        return True
    return False


def runtime_product_versions(runtime: dict[str, Any]) -> list[tuple[str, str]]:
    """Product semver fields only — skip envelope 3/4."""
    found: list[tuple[str, str]] = []
    raw = runtime.get("version")
    if raw is not None and not is_envelope_version(raw) and PRODUCT_VERSION.match(str(raw)):
        found.append(("runtime.version", str(raw)))
    binary = runtime.get("binary")
    if isinstance(binary, dict):
        bv = binary.get("version")
        if bv is not None and not is_envelope_version(bv) and PRODUCT_VERSION.match(str(bv)):
            found.append(("runtime.binary.version", str(bv)))
    plugin_version = runtime.get("plugin_version")
    if plugin_version is not None and not is_envelope_version(plugin_version) and PRODUCT_VERSION.match(str(plugin_version)):
        found.append(("runtime.plugin_version", str(plugin_version)))
    return found


def require_runtime_matches_package(staged: Path, version: str) -> None:
    path = staged / "runtime-manifest.json"
    if not path.is_file():
        raise PublishError("staged package missing runtime-manifest.json")
    runtime = _json_load(path)
    for name, value in runtime_product_versions(runtime):
        if value != version:
            raise PublishError(f"{name}={value} disagrees with plugin.json {version}")


def package_version(staged: Path) -> str:
    claude = staged / ".claude-plugin" / "plugin.json"
    if not claude.is_file():
        raise PublishError("staged package missing .claude-plugin/plugin.json")
    version = str(_json_load(claude).get("version") or "")
    if not version:
        raise PublishError("plugin.json has no version")
    return version


def require_package_catalog_name(staged: Path, catalog: str) -> None:
    claude = staged / ".claude-plugin" / "plugin.json"
    name = str(_json_load(claude).get("name") or "")
    if name != catalog:
        raise PublishError(f"plugin.json name {name!r} is not catalog {catalog!r}")
    codex = staged / ".codex-plugin" / "plugin.json"
    if not codex.is_file():
        return
    codex_name = str(_json_load(codex).get("name") or "")
    if codex_name != catalog:
        raise PublishError(f"codex plugin.json name {codex_name!r} is not catalog {catalog!r}")


def _git_tracks(marketplace: Path, rel: str) -> bool:
    proc = subprocess.run(
        ["git", "-C", str(marketplace), "ls-files", "--error-unmatch", "--", rel],
        capture_output=True,
        text=True,
    )
    return proc.returncode == 0


def marketplace_git_paths(marketplace: Path, catalog: str) -> list[str]:
    paths = [
        f"plugins/{catalog}",
        ".claude-plugin/marketplace.json",
        ".agents/plugins/marketplace.json",
    ]
    if (marketplace / "README.md").is_file():
        paths.append("README.md")
    leftover_rel = f"plugins/{leftover_cli_catalog(catalog)}"
    if (marketplace / leftover_rel).exists() or _git_tracks(marketplace, leftover_rel):
        paths.append(leftover_rel)
    return paths


def git_push_marketplace(marketplace: Path, catalog: str, version: str) -> None:
    subprocess.run(
        ["git", "-C", str(marketplace), "add", "-A", "--", *marketplace_git_paths(marketplace, catalog)],
        check=True,
    )
    status = subprocess.run(["git", "-C", str(marketplace), "status", "--porcelain"], capture_output=True, text=True, check=True)
    if not status.stdout.strip():
        raise PublishError("marketplace apply produced no git diff")
    subprocess.run(
        ["git", "-C", str(marketplace), "commit", "-m", f"release({catalog}): {version}"],
        check=True,
    )
    subprocess.run(["git", "-C", str(marketplace), "push", "origin", "HEAD"], check=True)


def publish(
    plugin_id: str,
    staged: Path,
    marketplace: Path,
    apply: bool,
    git_push: bool = False,
) -> dict[str, Any]:
    if plugin_id not in PLUGINS:
        raise PublishError(f"unknown plugin_id {plugin_id}")
    spec = PLUGINS[plugin_id]
    catalog = str(spec["catalog"])
    bin_name = str(spec["bin"])
    aud = spec["aud"]
    staged = staged.resolve()
    marketplace = marketplace.resolve()
    dest = marketplace / "plugins" / catalog
    leftover_dir = marketplace / "plugins" / leftover_cli_catalog(catalog)
    before = _tree_digest(dest)

    require_marketplace_key(marketplace)
    require_dual_bins(staged)
    version = package_version(staged)
    require_package_catalog_name(staged, catalog)
    bin_path = darwin_bin(staged, bin_name)
    if not os.access(bin_path, os.X_OK) and bin_path.suffix != ".py":
        bin_path.chmod(bin_path.stat().st_mode | 0o111)
    dump = commands_json(bin_path)
    dump2 = commands_json(bin_path)
    json_ver = str(dump.get("version") or "")
    json_ver2 = str(dump2.get("version") or "")
    if not json_ver:
        raise PublishError("commands --json has no version")
    if json_ver != json_ver2:
        raise PublishError("commands --json version is not stable across two invocations")
    ver_proc = run_cli(bin_path, ["--version"])
    if ver_proc.returncode == 0 and ver_proc.stdout.strip():
        first = staged_cli_version(ver_proc.stdout)
        second = staged_cli_version(run_cli(bin_path, ["--version"]).stdout)
        if first != second:
            raise PublishError("CLI --version is not stable across two invocations")
        if first != version:
            raise PublishError(f"CLI --version {first} disagrees with plugin.json {version}")
        if json_ver not in {version, first}:
            raise PublishError("commands --json version disagrees with plugin.json")
    else:
        if json_ver != version:
            raise PublishError(
                f"CLI has no --version and commands --json version {json_ver} disagrees with plugin.json {version}"
            )
    require_runtime_matches_package(staged, version)
    run_finalize_signed_hashes_check(staged)
    run_verify_package(staged)
    run_skill_surface(staged, plugin_id, bin_path)
    probe_backend(plugin_id, aud)

    result = {
        "ok": True,
        "plugin_id": plugin_id,
        "catalog": catalog,
        "version": version,
        "apply": apply,
        "marketplace": str(marketplace),
        "staged": str(staged),
    }
    if git_push and not apply:
        raise PublishError("--git-push requires --apply")
    if not apply:
        print(json.dumps(result, sort_keys=True))
        return result

    dest.parent.mkdir(parents=True, exist_ok=True)
    if dest.exists():
        shutil.rmtree(dest)
    shutil.copytree(staged, dest, symlinks=True)
    set_catalog_version(marketplace, catalog, version)
    rewrite_package_readme(dest)
    rewrite_marketplace_readme(marketplace)
    if leftover_dir.exists() and leftover_dir.resolve() != dest.resolve():
        shutil.rmtree(leftover_dir)
    after = _tree_digest(dest)
    if after == before:
        raise PublishError("apply wrote nothing to the marketplace tree")
    result["wrote"] = str(dest)
    if git_push:
        git_push_marketplace(marketplace, catalog, version)
        result["git_push"] = True
    print(json.dumps(result, sort_keys=True))
    return result


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--plugin-id", required=True)
    parser.add_argument("--staged", required=True, type=Path)
    parser.add_argument("--marketplace", required=True, type=Path)
    parser.add_argument("--apply", action="store_true")
    parser.add_argument("--git-push", action="store_true")
    args = parser.parse_args(argv)
    dest = args.marketplace.resolve() / "plugins" / str(PLUGINS.get(args.plugin_id, {}).get("catalog") or args.plugin_id)
    before = _tree_digest(dest)
    try:
        publish(args.plugin_id, args.staged, args.marketplace, args.apply, git_push=args.git_push)
    except PublishError:
        after = _tree_digest(dest)
        if after != before:
            print("publish-official-plugin: marketplace tree changed on failure", file=sys.stderr)
        return 2
    except SystemExit as exc:
        return int(exc.code) if isinstance(exc.code, int) else 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
