#!/usr/bin/env python3
"""Refuse an official plugin tag when required GitHub secret *names* are missing.

Does not print secret values. Exit 0 when every required name is present on the
repo; exit 2 otherwise.

Usage:
  python3 preflight-official-plugin-release.py --plugin-id easybooks
  python3 preflight-official-plugin-release.py --repo jackyzhang69/easybooks \\
      --secret PLUGINS_REPO_DEPLOY_KEY --secret APPLE_ID
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys


REQUIRED: dict[str, tuple[str, tuple[str, ...]]] = {
    "easybooks": (
        "jackyzhang69/easybooks",
        (
            "PLUGINS_REPO_DEPLOY_KEY",
            "APPLE_ID",
            "APPLE_APP_SPECIFIC_PASSWORD",
            "APPLE_TEAM_ID",
            "APPLE_SIGNING_IDENTITY",
            "CSC_LINK",
            "CSC_KEY_PASSWORD",
        ),
    ),
    "anychat": (
        "jackyzhang69/anychat",
        (
            "PLUGINS_REPO_DEPLOY_KEY",
            "APPLE_ID",
            "APPLE_APP_SPECIFIC_PASSWORD",
            "APPLE_TEAM_ID",
            "APPLE_SIGNING_IDENTITY",
            "CSC_LINK",
            "CSC_KEY_PASSWORD",
        ),
    ),
    "anydoc": (
        "jackyzhang69/anydoc",
        (
            "PLUGINS_REPO_DEPLOY_KEY",
            "APPLE_ID",
            "APPLE_APP_SPECIFIC_PASSWORD",
            "APPLE_TEAM_ID",
            "APPLE_SIGNING_IDENTITY",
            "CSC_LINK",
            "CSC_KEY_PASSWORD",
        ),
    ),
    "formbro": (
        "jackyzhang69/formbro-v3",
        (
            "PLUGINS_REPO_DEPLOY_KEY",
            "APPLE_ID",
            "APPLE_APP_SPECIFIC_PASSWORD",
            "APPLE_TEAM_ID",
            "APPLE_SIGNING_IDENTITY",
            "CSC_LINK",
            "CSC_KEY_PASSWORD",
        ),
    ),
    "anypdf": (
        "jackyzhang69/anybase",
        ("PLUGINS_REPO_DEPLOY_KEY",),
    ),
}


class PreflightError(SystemExit):
    def __init__(self, message: str) -> None:
        super().__init__(2)
        print(f"preflight-official-plugin-release: {message}", file=sys.stderr)


def list_secret_names(repo: str) -> set[str]:
    proc = subprocess.run(
        ["gh", "secret", "list", "--repo", repo, "--json", "name"],
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        raise PreflightError(f"gh secret list failed for {repo}")
    try:
        payload = json.loads(proc.stdout or "[]")
    except json.JSONDecodeError as exc:
        raise PreflightError("gh secret list returned non-JSON") from exc
    names = set()
    if isinstance(payload, list):
        for item in payload:
            if isinstance(item, dict) and item.get("name"):
                names.add(str(item["name"]))
    return names


def preflight(repo: str, required: tuple[str, ...]) -> None:
    present = list_secret_names(repo)
    missing = [name for name in required if name not in present]
    if missing:
        raise PreflightError(f"{repo} missing secret names: {', '.join(missing)}")
    print(json.dumps({"ok": True, "repo": repo, "required": list(required)}, sort_keys=True))


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--plugin-id")
    parser.add_argument("--repo")
    parser.add_argument("--secret", action="append", dest="secrets")
    args = parser.parse_args(argv)
    if args.plugin_id:
        if args.plugin_id not in REQUIRED:
            raise PreflightError(f"unknown plugin_id {args.plugin_id}")
        repo, required = REQUIRED[args.plugin_id]
        if args.repo:
            repo = args.repo
        preflight(repo, required)
        return 0
    if args.repo and args.secrets:
        preflight(args.repo, tuple(args.secrets))
        return 0
    raise PreflightError("need --plugin-id or --repo plus --secret")


if __name__ == "__main__":
    raise SystemExit(main())
