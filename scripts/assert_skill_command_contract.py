#!/usr/bin/env python3
"""EasyBooks skill contract: vault surface assert + --json placement.

The platform gate (scripts/official-plugin/assert-skill-surface.py, byte-identical
to platform governance/scripts/assert-skill-surface.py) is the CLI↔skill contract.
This wrapper adds the EasyBooks-only rule that `--json` precedes `doctor` /
`whoami` / `login` (those subcommands declare `--json` as a payload flag).
"""

from __future__ import annotations

import json
from pathlib import Path
import re
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[1]
SKILL_ROOTS = (ROOT / ".claude/skills",)
SUBCOMMANDS_WITH_TOP_LEVEL_JSON = ("doctor", "whoami", "login")
SURFACE = ROOT / "scripts/official-plugin/assert-skill-surface.py"


def _bin_path() -> Path | None:
    candidates: list[Path] = []
    try:
        meta = json.loads(
            subprocess.check_output(
                ["cargo", "metadata", "--format-version", "1", "--no-deps"],
                cwd=ROOT,
                text=True,
            )
        )
        target_dir = Path(meta["target_directory"])
        candidates.extend(
            [
                target_dir / "release" / "easybooks",
                target_dir / "debug" / "easybooks",
            ]
        )
    except (OSError, subprocess.CalledProcessError, json.JSONDecodeError, KeyError):
        pass
    candidates.extend(
        [
            ROOT / "target/release/easybooks",
            ROOT / "target/debug/easybooks",
        ]
    )
    seen: set[Path] = set()
    for candidate in candidates:
        if candidate in seen:
            continue
        seen.add(candidate)
        if candidate.is_file():
            return candidate
    return None


def vault_surface() -> list[str]:
    binary = _bin_path()
    skills = ROOT / ".claude/skills/easybooks"
    if binary is None:
        return [
            "assert-skill-surface.py skipped: no built easybooks binary under target/"
            " (build first; CI builds before this wrapper)"
        ]
    proc = subprocess.run(
        [sys.executable, str(SURFACE), "--bin", str(binary.resolve()), "--skills", str(skills)],
        check=False,
        capture_output=True,
        text=True,
    )
    sys.stdout.write(proc.stdout)
    sys.stderr.write(proc.stderr)
    if proc.returncode != 0:
        return [f"assert-skill-surface.py exited {proc.returncode}"]
    return []


def json_placement() -> list[str]:
    failures: list[str] = []
    checked_files = 0
    for skill_root in SKILL_ROOTS:
        for path in sorted(skill_root.rglob("*.md")):
            checked_files += 1
            source = path.read_text(encoding="utf-8")
            for command in SUBCOMMANDS_WITH_TOP_LEVEL_JSON:
                pattern = re.compile(rf"\b{command}\s+--json\b")
                for match in pattern.finditer(source):
                    line = source.count("\n", 0, match.start()) + 1
                    failures.append(
                        f"{path.relative_to(ROOT)}:{line}: --json must precede the {command} subcommand"
                    )

    required_examples = {
        ROOT / ".claude/skills/easybooks/references/connect.md": (
            '"$EASYBOOKS_BIN" --json doctor --no-fetch',
        ),
        ROOT / ".claude/skills/easybooks/SKILL.md": (
            "easybooks --json doctor",
        ),
    }
    for path, examples in required_examples.items():
        source = path.read_text(encoding="utf-8")
        for example in examples:
            if example not in source:
                failures.append(f"{path.relative_to(ROOT)}: missing required example: {example}")
    return failures


def main() -> int:
    failures = json_placement()
    surface_failures = vault_surface()
    # If the binary is missing (local `python3 scripts/assert_skill_command_contract.py`
    # before cargo build), keep the --json placement gate and warn.
    if surface_failures and surface_failures[0].startswith("assert-skill-surface.py skipped"):
        print(surface_failures[0], file=sys.stderr)
        surface_failures = []
    failures.extend(surface_failures)
    result = {
        "ok": not failures,
        "failures": failures,
    }
    print(json.dumps(result, sort_keys=True))
    return 0 if result["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())
