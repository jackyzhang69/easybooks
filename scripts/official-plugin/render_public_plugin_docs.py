#!/usr/bin/env python3
"""Render public plugin marketplace and package README from delivery/hosts.yaml.

Usage:
  render_public_plugin_docs.py marketplace --marketplace DIR [--write]
  render_public_plugin_docs.py package --staged DIR [--write]
  render_public_plugin_docs.py verify-marketplace --marketplace DIR
  render_public_plugin_docs.py verify-package --staged DIR

Without --write, prints the rendered document to stdout.
verify-* exits 2 when on-disk README differs from the render (byte compare).
Does not push remotes or mutate production marketplace without the caller.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any

try:
    import yaml
except ImportError as exc:  # pragma: no cover
    raise SystemExit(f"PyYAML required: {exc}") from exc

SCRIPT_DIR = Path(__file__).resolve().parent
GOVERNANCE_ROOT = SCRIPT_DIR.parents[1]
DEFAULT_HOSTS = GOVERNANCE_ROOT / "delivery" / "hosts.yaml"

MARKER_BEGIN = "<!-- generated-by: render_public_plugin_docs.py ; do not edit -->"
MARKER_END = "<!-- end generated-by: render_public_plugin_docs.py -->"


class RenderError(Exception):
    pass


def load_hosts(path: Path) -> dict[str, Any]:
    if not path.is_file():
        raise RenderError(f"hosts.yaml missing: {path}")
    data = yaml.safe_load(path.read_text(encoding="utf-8"))
    if not isinstance(data, dict) or data.get("schema") != "plugin-hosts-v1":
        raise RenderError("hosts.yaml must declare schema: plugin-hosts-v1")
    if not isinstance(data.get("recipes"), list):
        raise RenderError("hosts.yaml recipes must be a list")
    return data


def _json_load(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def catalog_entries(marketplace: Path) -> list[dict[str, Any]]:
    claude = marketplace / ".claude-plugin" / "marketplace.json"
    agents = marketplace / ".agents" / "plugins" / "marketplace.json"
    if not claude.is_file() or not agents.is_file():
        raise RenderError("marketplace missing both catalog files")
    left = _json_load(claude)
    right = _json_load(agents)
    left_plugins = left.get("plugins") or []
    right_plugins = right.get("plugins") or []
    if not isinstance(left_plugins, list) or not isinstance(right_plugins, list):
        raise RenderError("marketplace plugins must be lists")

    def norm(rows: list[Any]) -> dict[str, dict[str, Any]]:
        out: dict[str, dict[str, Any]] = {}
        for row in rows:
            if not isinstance(row, dict):
                raise RenderError("marketplace plugin row must be an object")
            name = str(row.get("name") or "")
            if not name:
                raise RenderError("marketplace plugin row missing name")
            if name in out:
                raise RenderError(f"duplicate marketplace plugin name {name}")
            out[name] = row
        return out

    a = norm(left_plugins)
    b = norm(right_plugins)
    if set(a) != set(b):
        raise RenderError(
            f"marketplace catalogs disagree on plugin names: {sorted(set(a) ^ set(b))}"
        )
    for name in sorted(a):
        if str(a[name].get("version") or "") != str(b[name].get("version") or ""):
            raise RenderError(f"marketplace catalogs disagree on version for {name}")
        if str(a[name].get("source") or "") != str(b[name].get("source") or ""):
            raise RenderError(f"marketplace catalogs disagree on source for {name}")
    # Prefer claude catalog order, then any name-stable sort for determinism.
    order = [str(row.get("name")) for row in left_plugins]
    return [a[name] for name in order]


def _display_name(plugin_id: str) -> str:
    return plugin_id


def _platforms_cell() -> str:
    return "macOS (`arm64`), Windows (`x64`), Linux (`x64`)"


def _description_cell(row: dict[str, Any]) -> str:
    desc = str(row.get("description") or "").strip()
    if not desc:
        return "_see package README_"
    # Keep table cells single-line.
    return " ".join(desc.split())


# Public README safety phrases that must not disappear on a one-plugin republish.
_SAFETY_MARKERS = (
    "stops safely before submit",
    "stopping safely before final submission",
    "before final submission",
    "before submit",
)


def existing_readme_table_descriptions(marketplace: Path) -> dict[str, str]:
    """Parse current marketplace README plugin-table descriptions, if any."""
    path = marketplace / "README.md"
    if not path.is_file():
        return {}
    out: dict[str, str] = {}
    # | [`formbro`](./plugins/formbro) | 1.9.4 | macOS (`arm64`), Windows (`x64`) | desc |
    pattern = re.compile(
        r"^\|\s*\[`([^`]+)`\][^|]*\|\s*[^|]+\|\s*[^|]+\|\s*(.*?)\s*\|\s*$"
    )
    for line in path.read_text(encoding="utf-8").splitlines():
        match = pattern.match(line)
        if not match:
            continue
        name = match.group(1).strip()
        desc = " ".join(match.group(2).split())
        if name and desc:
            out[name] = desc
    return out


def _has_safety_marker(text: str) -> bool:
    lower = text.lower()
    return any(marker in lower for marker in _SAFETY_MARKERS)


def package_table_description(
    marketplace: Path,
    row: dict[str, Any],
    existing: dict[str, str] | None = None,
) -> str:
    """Prefer shipped package plugin.json; never drop prior README safety wording.

    Regenerating the marketplace README must not erase product safety sentences
    that still live only in the public table (e.g. FormBro stops-before-submit)
    when a one-plugin publish refreshes every row.
    """
    name = str(row.get("name") or "")
    candidate = ""
    if name:
        plugin_json = (
            marketplace / "plugins" / name / ".claude-plugin" / "plugin.json"
        )
        if plugin_json.is_file():
            try:
                meta = _json_load(plugin_json)
            except json.JSONDecodeError as exc:
                raise RenderError(
                    f"plugins/{name}/.claude-plugin/plugin.json is not JSON: {exc}"
                ) from exc
            candidate = str(meta.get("description") or "").strip()
    if not candidate:
        candidate = str(row.get("description") or "").strip()
    if candidate:
        candidate = " ".join(candidate.split())
    else:
        candidate = "_see package README_"

    prior = (existing or {}).get(name, "")
    if prior and _has_safety_marker(prior) and not _has_safety_marker(candidate):
        return prior
    return candidate


def render_marketplace_readme(marketplace: Path, hosts: dict[str, Any]) -> str:
    plugins = catalog_entries(marketplace)
    existing_desc = existing_readme_table_descriptions(marketplace)
    recipes = hosts.get("recipes") or []
    public_marketplace = [
        r for r in recipes if r.get("public_recipe") and r.get("kind") == "marketplace_plugin"
    ]
    public_generic = [
        r
        for r in recipes
        if r.get("public_recipe") and r.get("kind") == "agent_skills"
    ]
    for recipe in public_marketplace:
        if not recipe.get("steps"):
            raise RenderError(
                f"recipe {recipe.get('id')} has public_recipe but no install steps"
            )

    lines: list[str] = [
        MARKER_BEGIN,
        "# jacky-plugins",
        "",
        "Official Jacky plugins for agents that speak **Agent Skills** (and for hosts with a plugin marketplace).",
        "",
        "One skill tree per product under `plugins/<plugin_id>/skills/`. The native CLI lives beside it under `bin/`. Marketplace install commands are convenience; any agent that can attach a `SKILL.md` folder or the full plugin tree can use the same package.",
        "",
        "## Available Plugins",
        "",
        "| Plugin | Version | Platforms | Description |",
        "|---|---|---|---|",
    ]
    for row in plugins:
        name = str(row["name"])
        version = str(row.get("version") or "")
        if not version:
            raise RenderError(f"marketplace entry {name} has no version")
        source = str(row.get("source") or f"./plugins/{name}")
        link = f"[`{name}`]({source})"
        lines.append(
            f"| {link} | {version} | {_platforms_cell()} | {package_table_description(marketplace, row, existing_desc)} |"
        )

    lines.extend(["", "## Installation", ""])

    if public_marketplace:
        lines.append("### Hosts with a plugin marketplace")
        lines.append("")
        for recipe in public_marketplace:
            title = str(recipe.get("title") or recipe.get("id"))
            lines.append(f"#### {title}")
            lines.append("")
            lines.append("```bash")
            for step in recipe.get("steps") or []:
                lines.append(str(step).replace("<plugin_id>", "formbro"))
            lines.append("```")
            lines.append("")
            lines.append(
                "Replace `formbro` with the plugin id from the table (`anypdf`, `anyweb`, …)."
            )
            lines.append("")
        install_note = str((hosts.get("portable") or {}).get("marketplace_install_note") or "").strip()
        if not install_note:
            raise RenderError("hosts.yaml portable.marketplace_install_note is required")
        lines.append(install_note)
        lines.append("")

    if public_generic:
        lines.append("### Any Agent Skills–compatible agent")
        lines.append("")
        lines.append(
            "Cherry Studio, Cursor, Gemini CLI, and other Agent Skills hosts do **not** need a Jacky-specific `plugin install` command. Attach the package the host already understands:"
        )
        lines.append("")
        for recipe in public_generic:
            for step in recipe.get("steps") or []:
                lines.append(f"- {step}")
        lines.append("")
        lines.append(
            "After attach, load `skills/<plugin_id>/SKILL.md`. Product commands use the repaired CLI under `~/.jackyzhang.app/plugins/<plugin_id>/current`, never whatever binary happens to be first on `PATH`."
        )
        lines.append("")

    lines.extend(
        [
            "## Before first use — platform prerequisites",
            "",
            "Read the product's `skills/<plugin_id>/SKILL.md` first. Two platform "
            "prerequisites account for most \"the plugin is broken\" reports:",
            "",
            "- **macOS — Full Disk Access belongs to the app that runs your agent, "
            "not to the plugin binary.** Products that read data kept by another "
            "app (AnyChat reads the chat archive) are credited by macOS to the "
            "host app that launched the CLI. System Settings → Privacy & Security "
            "→ Full Disk Access → add that agent host (DeepSeek Harness, Claude, "
            "ChatGPT, Codex, Cherry Studio, …) → quit and reopen it. Granting the "
            "bundled `anychat` binary itself does not work.",
            "- **Windows — one supported source-app build, permission prompt "
            "allowed, source app signed in.** Install only the build the product "
            "lists, approve the UAC prompt when setup asks, and keep the source "
            "app signed in and running during setup. If setup reports a "
            "key/permission failure, collect the product's `doctor --json` and "
            "send it through Tell Jacky; do not reinstall the source app and do "
            "not delete chat data.",
            "",
            "Update the product CLI before diagnosing — for AnyChat: "
            "`anychat update --json`, then `anychat doctor --json`. Current "
            "releases refresh a stale local authorization cache during upgrade.",
            "",
        ]
    )

    lines.extend(
        [
            "## Package shape",
            "",
            "Each plugin under `plugins/<plugin_id>/` contains:",
            "",
            "- `.claude-plugin/plugin.json` and `.codex-plugin/plugin.json` — host marketplace metadata when that host uses a catalog",
            "- `skills/<plugin_id>/SKILL.md` — the single Agent Skills router; playbooks live in `references/`",
            "- `bin/<platform>/` — native CLI for `darwin-arm64` and `win32-x64`, with `.sha256` sidecars",
            "- `runtime-manifest.json` — platform map and package verification",
            "- `README.md` — thin package card (generated; details live in the skill)",
            "",
            "## Hard rules (user-facing)",
            "",
            "- **One Portal sign-in.** Create a durable user token at [jackyzhang.app/account/tokens](https://jackyzhang.app/account/tokens). The host agent connects for you; the token is piped on stdin (`login --token-stdin`), never placed on the command line.",
            "- **Shared credential slot.** Successful connect writes mode-`0600` `~/.jackyzhang.app/token/user.json` (Windows: `%USERPROFILE%\\.jackyzhang.app\\token\\user.json`). Other official plugins reuse that slot.",
            "- **Local native CLI.** Skills drive the bundled binary. Documents and site sessions that the product keeps local stay on this machine except for the official product backends the skill names.",
            "- **No secrets in chat artifacts.** Do not paste tokens into skill text, logs, screenshots, or argv.",
            "",
            "## License",
            "",
            "MIT (see [LICENSE](./LICENSE)).",
            "",
            MARKER_END,
            "",
        ]
    )
    return "\n".join(lines)


def render_package_readme(staged: Path, hosts: dict[str, Any] | None = None) -> str:
    del hosts  # reserved; package card stays product-local from plugin.json
    claude = staged / ".claude-plugin" / "plugin.json"
    if not claude.is_file():
        raise RenderError("staged package missing .claude-plugin/plugin.json")
    meta = _json_load(claude)
    name = str(meta.get("name") or "").strip()
    version = str(meta.get("version") or "").strip()
    description = str(meta.get("description") or "").strip()
    if not name or not version or not description:
        raise RenderError("plugin.json needs name, version, and description")
    skill = staged / "skills" / name / "SKILL.md"
    if not skill.is_file():
        raise RenderError(f"missing skills/{name}/SKILL.md")
    whats_new = f"skills/{name}/references/whats-new.md"
    lines = [
        MARKER_BEGIN,
        f"# {name}",
        "",
        f"Version `{version}`.",
        "",
        description,
        "",
        "Platforms: macOS Apple Silicon (`darwin-arm64`), Windows x64 (`win32-x64`), Linux x64 (`linux-x64`).",
        "",
        f"Load [`skills/{name}/SKILL.md`](skills/{name}/SKILL.md) first. Connect and product verbs are defined there and under `skills/{name}/references/`.",
        "",
        "This package is an **Agent Skills** tree plus a native CLI. Any agent that can attach a skill folder or this full plugin directory can use it — marketplace `plugin install` is optional convenience, not a requirement.",
        "",
        "- Full tree (preferred): this directory, including `bin/` and `runtime-manifest.json`, then `bin/<platform>/<cli> doctor --repair-install`.",
        f"- Skill-only attach: only after `~/.jackyzhang.app/plugins/{name}/current` already holds the matching CLI.",
        "",
        f"What changed in this version: [`{whats_new}`]({whats_new}).",
        "",
        MARKER_END,
        "",
    ]
    return "\n".join(lines)


def _write_or_print(path: Path | None, text: str, write: bool) -> None:
    if write:
        if path is None:
            raise RenderError("--write requires an output path")
        path.write_text(text, encoding="utf-8")
        return
    sys.stdout.write(text)


def _verify(path: Path, expected: str) -> None:
    if not path.is_file():
        raise RenderError(f"missing README to verify: {path}")
    actual = path.read_text(encoding="utf-8")
    if actual != expected:
        raise RenderError(f"README drift (regenerate): {path}")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--hosts",
        type=Path,
        default=DEFAULT_HOSTS,
        help="path to delivery/hosts.yaml",
    )
    sub = parser.add_subparsers(dest="cmd", required=True)

    p_m = sub.add_parser("marketplace", help="render marketplace README.md")
    p_m.add_argument("--marketplace", required=True, type=Path)
    p_m.add_argument("--write", action="store_true")

    p_p = sub.add_parser("package", help="render package README.md")
    p_p.add_argument("--staged", required=True, type=Path)
    p_p.add_argument("--write", action="store_true")

    p_vm = sub.add_parser("verify-marketplace", help="fail if marketplace README drifted")
    p_vm.add_argument("--marketplace", required=True, type=Path)

    p_vp = sub.add_parser("verify-package", help="fail if package README drifted")
    p_vp.add_argument("--staged", required=True, type=Path)

    args = parser.parse_args(argv)
    try:
        hosts = load_hosts(args.hosts.resolve())
        if args.cmd == "marketplace":
            text = render_marketplace_readme(args.marketplace.resolve(), hosts)
            _write_or_print(args.marketplace.resolve() / "README.md", text, args.write)
        elif args.cmd == "package":
            staged = args.staged.resolve()
            text = render_package_readme(staged, hosts)
            _write_or_print(staged / "README.md", text, args.write)
        elif args.cmd == "verify-marketplace":
            marketplace = args.marketplace.resolve()
            _verify(marketplace / "README.md", render_marketplace_readme(marketplace, hosts))
        elif args.cmd == "verify-package":
            staged = args.staged.resolve()
            _verify(staged / "README.md", render_package_readme(staged, hosts))
        else:
            raise RenderError(f"unknown command {args.cmd}")
    except RenderError as exc:
        print(f"render_public_plugin_docs: {exc}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
