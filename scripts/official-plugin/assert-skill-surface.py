#!/usr/bin/env python3
"""Fail-closed skill ↔ CLI surface contract.

Usage:
  python3 assert-skill-surface.py --bin <absolute path of the binary being published> \\
                                  --skills <skills/<product> dir> [--write-surface]

This script is the mechanical substitute for “skills cannot stay frozen when
the CLI surface changes”. It never consults PATH. Missing `commands --json`
is FAIL, never skip.

Dump (`<bin> commands --json`, schema `plugin-cli-commands-v2`):
  {"schema","bin","version","global_args":[Arg],
   "commands":[{"path":[..],"summary":"..","detail":".."?,"args":[Arg]}]}
  Arg = {"name","kind":"option"|"flag"|"positional","required":bool,
         "values"?:[..],"help"?:".."}. Every command needs a non-empty summary.
  Unknown extra keys are allowed.

Generated reference:
  `references/command-surface.md` is rendered from the dump by this script.
  `--write-surface` writes it, then runs every check. Without the flag the
  file must equal the rendering byte for byte (FAIL
  `command-surface.md is stale; rerun with --write-surface`).

Direction B (coverage, required):
  Every public commands[].path must appear at least once as a contiguous
  token sequence in SKILL.md or references/*.md, EXCEPT the generated
  command-surface.md (it lists everything by construction). Matching accepts
  backtick-quoted or unquoted joined paths with word boundaries.

Direction A (backtick contract, required — not a no-op):
  A backtick-quoted token is treated as a CLI invocation when:
    1. it starts with the binary name, `$BIN`, `$<PRODUCT>_BIN`,
       `"$BIN"`, `"$<PRODUCT>_BIN"`, or `& $env:<PRODUCT>_BIN`, or
    2. the first non-flag token is a known public top-level command AND the
       binary name appears on the same line.
  Remaining tokens up to the first flag (`-…`) form a command path. That
  path is valid if it equals a public path, is a prefix of a public path,
  or a public path is a prefix of it (over-specified subcommand). FAIL if a
  backtick invocation names a command that is not public. When the path
  equals a public path (or over-specifies one), every `--flag` token (cut at
  `=`, up to the first shell operator) must be one of that command's args,
  a global arg, or `--help`/`--version` (FAIL `unknown flag <flag> for <path>`).

Required package docs under references/:
  get-started.md exists; whats-new.md exists and its first heading line names
  the dump version; the router SKILL.md contains the literals
  `jz.plugin.update_notice.v1` and `doctor --repair-install`;
  cold-read.json (`jz.plugin.cold_read.v1`) is the answer of a fresh reader
  who saw only the docs: `plugin_id`, `version` equal to the dump version,
  `capabilities` = [{goal, command:[..], flags:[..]}], `unclear` == []; every
  public path appears as some `command`, every `command` is a public path,
  every flag exists for its command.

Unquoted `<bin> <subcommand>` mentions (not in backticks) are WARN only:
they give false confidence. Skills should backtick-quote live invocations.
WARN does not fail the process.

Trigger preservation (if references/triggers.md exists):
  Each markdown bullet in that file must appear in SKILL.md (frontmatter +
  the first 80 lines). This replaces “fresh-session trigger evaluation”.

Hidden/admin/debug commands must not appear in commands --json; this script
does not second-guess the CLI dump. Volatile catalogs (form lists) are not
part of the dump by contract.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path


SCHEMA = "plugin-cli-commands-v2"
COLD_READ_SCHEMA = "jz.plugin.cold_read.v1"
FIRST_SCREEN_LINES = 80
HIDDEN_TOP_LEVEL = {"admin", "help", "debug", "internal"}
ARG_KINDS = ("option", "flag", "positional")
BUILTIN_FLAGS = {"--help", "--version"}
SURFACE_NAME = "command-surface.md"
SURFACE_HEADER = "<!-- generated-by: assert-skill-surface.py ; do not edit -->"
ROUTER_LITERALS = ("jz.plugin.update_notice.v1", "doctor --repair-install")
SHELL_OPERATORS = {"|", "||", "&&", "&", ";", ">", ">>", "<", "2>", "2>&1", "&>"}
FLAG_RE = re.compile(r"^--[A-Za-z0-9][\w-]*$")

BIN_ENV_RE = re.compile(
    r"""^(?:
            \$\{?(?:BIN|[A-Z][A-Z0-9_]*_BIN)\}? |
            "(?:\$\{?(?:BIN|[A-Z][A-Z0-9_]*_BIN)\}?)" |
            (?:\&\s+)?\$env:[A-Z][A-Z0-9_]*_BIN
        )$""",
    re.VERBOSE,
)


class Check:
    def __init__(self) -> None:
        self.failures: list[str] = []
        self.warnings: list[str] = []

    def fail(self, msg: str) -> None:
        if msg not in self.failures:
            self.failures.append(msg)

    def warn(self, msg: str) -> None:
        self.warnings.append(msg)


class Surface:
    """What a valid dump says: public paths, flags per command, global flags."""

    def __init__(self) -> None:
        self.paths: list[list[str]] = []
        self.flags: dict[tuple[str, ...], set[str]] = {}
        self.global_flags: set[str] = set()

    def allowed_flags(self, path: list[str]) -> set[str]:
        return self.flags.get(tuple(path), set()) | self.global_flags | BUILTIN_FLAGS


def die(msg: str, code: int = 2) -> None:
    print(f"assert-skill-surface: {msg}", file=sys.stderr)
    raise SystemExit(code)


def load_markdown(skills_dir: Path) -> tuple[Path, str, str]:
    """Router text and the narrative (router + references, minus the generated surface)."""
    skill = skills_dir / "SKILL.md"
    if not skill.is_file():
        die(f"missing router SKILL.md under {skills_dir}")
    parts = [skill.read_text(encoding="utf-8")]
    refs = skills_dir / "references"
    if refs.is_dir():
        for path in sorted(refs.rglob("*.md")):
            if path.name == "SKILL.md":
                die(f"SKILL.md is not allowed under references/: {path}")
            if path.name == SURFACE_NAME and path.parent == refs:
                continue
            parts.append(path.read_text(encoding="utf-8"))
    combined = "\n".join(parts)
    router = parts[0]
    return skill, router, combined


def run_commands_json(bin_path: Path) -> dict:
    if not bin_path.is_absolute():
        die(f"--bin must be an absolute path, got {bin_path}")
    if not bin_path.is_file():
        die(f"--bin is not a file: {bin_path}", 1)
    argv = [str(bin_path), "commands", "--json"]
    # Source-tree Python clients are not execve-able on every host; invoke
    # them through this interpreter. Missing `commands --json` is still FAIL.
    if bin_path.suffix == ".py":
        argv = [sys.executable, str(bin_path), "commands", "--json"]
    try:
        proc = subprocess.run(
            argv,
            check=False,
            capture_output=True,
            text=True,
        )
    except OSError as exc:
        die(f"could not execute {bin_path}: {exc}", 1)
    if proc.returncode != 0:
        die(
            "commands --json failed "
            f"(exit {proc.returncode}; missing command is FAIL, never skip)\n"
            f"stdout: {proc.stdout[:500]}\nstderr: {proc.stderr[:500]}",
            1,
        )
    raw = proc.stdout.strip()
    if not raw:
        die("commands --json produced empty stdout", 1)
    try:
        payload = json.loads(raw)
    except json.JSONDecodeError as exc:
        die(f"commands --json is not JSON: {exc}\n{raw[:500]}", 1)
    if not isinstance(payload, dict):
        die("commands --json must be a JSON object", 1)
    return payload


def validate_args(owner: str, args: object, check: Check) -> set[str]:
    """Validate an Arg list; returns the arg names that are well formed."""
    names: set[str] = set()
    if not isinstance(args, list):
        check.fail(f"{owner}: args must be an array")
        return names
    for item in args:
        if (
            not isinstance(item, dict)
            or not isinstance(item.get("name"), str)
            or not item["name"]
            or item.get("kind") not in ARG_KINDS
            or not isinstance(item.get("required"), bool)
            or ("values" in item and not (
                isinstance(item["values"], list) and all(isinstance(v, str) for v in item["values"])
            ))
            or ("help" in item and not isinstance(item["help"], str))
        ):
            check.fail(
                f"{owner}: arg must be {{name, kind: option|flag|positional, required: bool}}: {item!r}"
            )
            continue
        names.add(item["name"])
    return names


def validate_schema(payload: dict, bin_path: Path, check: Check) -> Surface:
    surface = Surface()
    if payload.get("schema") != SCHEMA:
        check.fail(f"schema is {payload.get('schema')!r}, expected {SCHEMA!r}")
    if not isinstance(payload.get("bin"), str) or not payload["bin"]:
        check.fail("bin is missing")
    if not isinstance(payload.get("version"), str) or not payload["version"]:
        check.fail("version is missing")
    if "global_args" not in payload:
        check.fail("global_args is missing (use [] when there are none)")
    else:
        surface.global_flags = validate_args("global_args", payload["global_args"], check)
    commands = payload.get("commands")
    if not isinstance(commands, list) or not commands:
        check.fail("commands must be a non-empty array")
        return surface
    seen: set[tuple[str, ...]] = set()
    for item in commands:
        if not isinstance(item, dict) or not isinstance(item.get("path"), list):
            check.fail(f"command entry is not {{path: [...]}}: {item!r}")
            continue
        path = [str(part) for part in item["path"]]
        if not path or any(not part for part in path):
            check.fail(f"empty command path: {item!r}")
            continue
        joined = " ".join(path)
        if path[0] in HIDDEN_TOP_LEVEL:
            check.fail(f"hidden/admin/debug path leaked into public dump: {joined}")
            continue
        key = tuple(path)
        if key in seen:
            check.fail(f"duplicate command path: {joined}")
            continue
        seen.add(key)
        if not isinstance(item.get("summary"), str) or not item["summary"].strip():
            check.fail(f"missing summary: {joined}")
        if "detail" in item and not isinstance(item["detail"], str):
            check.fail(f"detail must be a string: {joined}")
        surface.flags[key] = validate_args(joined, item.get("args"), check)
        surface.paths.append(path)
    return surface


# ----------------------------------------------------------------- generated surface


def _fold(text: object) -> str:
    return re.sub(r"\s*\n\s*", " ", str(text).replace("\r\n", "\n").strip())


def _cell(text: object) -> str:
    return _fold(text).replace("|", "\\|")


def _arg_table(args: list[dict]) -> list[str]:
    rows = [
        "| Argument | Kind | Required | Values | Meaning |",
        "|---|---|---|---|---|",
    ]
    for arg in args:
        values = ", ".join(f"`{_cell(v)}`" for v in arg.get("values") or [])
        rows.append(
            f"| `{_cell(arg['name'])}` | {arg['kind']} | {'yes' if arg['required'] else 'no'} "
            f"| {values} | {_cell(arg.get('help') or '')} |"
        )
    return rows


def render_surface(payload: dict) -> str:
    """The exact text of references/command-surface.md for a valid dump."""
    bin_name = payload["bin"]
    lines = [
        SURFACE_HEADER,
        f"# {bin_name} command reference",
        "",
        f"Generated from `{bin_name} commands --json` (version `{payload['version']}`). "
        "Every public command is listed.",
        "",
    ]
    if payload["global_args"]:
        lines += ["## Global options", "", *_arg_table(payload["global_args"]), ""]
    for command in payload["commands"]:
        lines += [f"## `{bin_name} {' '.join(command['path'])}`", "", _fold(command["summary"]), ""]
        detail = str(command.get("detail") or "").replace("\r\n", "\n").strip()
        if detail:
            lines += [detail, ""]
        if command["args"]:
            lines += [*_arg_table(command["args"]), ""]
        else:
            lines += ["No arguments.", ""]
    return "\n".join(lines).rstrip("\n") + "\n"


def check_surface_file(skills_dir: Path, payload: dict, check: Check) -> None:
    path = skills_dir / "references" / SURFACE_NAME
    expected = render_surface(payload).encode("utf-8")
    if not path.is_file() or path.read_bytes() != expected:
        check.fail("command-surface.md is stale; rerun with --write-surface")


# ----------------------------------------------------------------- directions A and B


def word_boundary_present(haystack: str, needle: str) -> bool:
    return re.search(r"(?<![\w-])" + re.escape(needle) + r"(?![\w-])", haystack) is not None


def direction_b(paths: list[list[str]], combined: str, check: Check) -> None:
    for path in paths:
        joined = " ".join(path)
        if word_boundary_present(combined, joined):
            continue
        check.fail(
            f"Direction B: public command `{joined}` is not mentioned in SKILL.md or references/"
        )


def tokenize_backtick(span: str) -> list[str]:
    span = span.strip()
    span = span.replace("\n", " ")
    # Strip wrapping quotes left after markdown extraction.
    parts = re.findall(r"""[^\s"'`]+|"[^"]*"|'[^']*'""", span)
    out: list[str] = []
    for part in parts:
        if (part.startswith('"') and part.endswith('"')) or (
            part.startswith("'") and part.endswith("'")
        ):
            part = part[1:-1]
        if part:
            out.append(part)
    return out


def looks_like_bin_token(token: str, bin_name: str) -> bool:
    token = token.strip()
    if token in {bin_name, f"{bin_name}.exe"}:
        return True
    if BIN_ENV_RE.match(token):
        return True
    if token.endswith(f"/{bin_name}") or token.endswith(f"\\{bin_name}") or token.endswith(
        f"\\{bin_name}.exe"
    ):
        return True
    return False


def path_from_tokens(tokens: list[str], bin_name: str, top_level: set[str]) -> list[str] | None:
    if not tokens:
        return None
    idx = 0
    if looks_like_bin_token(tokens[0], bin_name):
        idx = 1
    elif tokens[0] in top_level:
        idx = 0
    else:
        return None
    path: list[str] = []
    while idx < len(tokens):
        tok = tokens[idx]
        if tok.startswith("-"):
            break
        if tok.startswith("<") or tok.startswith("$") or tok in {"…", "..."}:
            break
        path.append(tok)
        idx += 1
    return path or None


def path_known(candidate: list[str], public: list[list[str]]) -> bool:
    if not candidate:
        return False
    for path in public:
        if path == candidate:
            return True
        if path[: len(candidate)] == candidate:
            return True
        if candidate[: len(path)] == path:
            return True
    return False


def public_match(candidate: list[str], public: list[list[str]]) -> list[str] | None:
    """The longest public path equal to, or a prefix of, the candidate."""
    best: list[str] | None = None
    for path in public:
        if candidate[: len(path)] == path and (best is None or len(path) > len(best)):
            best = path
    return best


def invocation_flags(tokens: list[str]) -> list[str]:
    """`--flag` tokens of one invocation (cut at `=`), up to the first shell operator."""
    flags: list[str] = []
    for tok in tokens:
        if tok in SHELL_OPERATORS:
            break
        if tok.startswith("--"):
            name = tok.split("=", 1)[0]
            if FLAG_RE.match(name):
                flags.append(name)
    return flags


def direction_a(
    combined: str,
    router_lines: list[str],
    bin_name: str,
    surface: Surface,
    check: Check,
) -> None:
    public = surface.paths
    top_level = {path[0] for path in public}
    backtick_re = re.compile(r"`([^`]+)`")
    line_starts = [0]
    for line in combined.splitlines(True):
        line_starts.append(line_starts[-1] + len(line))

    def line_of(pos: int) -> str:
        # Approximate: find the line containing pos.
        running = 0
        for line in combined.splitlines():
            nxt = running + len(line) + 1
            if pos < nxt:
                return line
            running = nxt
        return ""

    for match in backtick_re.finditer(combined):
        span = match.group(1)
        tokens = tokenize_backtick(span)
        if not tokens:
            continue
        line = line_of(match.start())
        starts_with_bin = looks_like_bin_token(tokens[0], bin_name)
        first_cmd = None
        # Skip leading flags after an optional bin token.
        scan = tokens[1:] if starts_with_bin else tokens
        for tok in scan:
            if tok.startswith("-"):
                continue
            first_cmd = tok
            break
        same_line_has_bin = bin_name in line or looks_like_bin_token(
            tokens[0], bin_name
        )
        if not starts_with_bin and not (
            first_cmd in top_level and same_line_has_bin
        ):
            continue
        path = path_from_tokens(tokens, bin_name, top_level)
        if not path:
            continue
        if not path_known(path, public):
            check.fail(
                "Direction A: backtick `"
                + span
                + f"` looks like `{bin_name} {' '.join(path)}` "
                "but that path is not in commands --json"
            )
            continue
        matched = public_match(path, public)
        if matched is None:
            continue
        allowed = surface.allowed_flags(matched)
        for flag in invocation_flags(tokens):
            if flag not in allowed:
                check.fail(f"unknown flag {flag} for {' '.join(matched)}")

    # Unquoted `<bin> <subcommand>` — warn only.
    unquoted = re.compile(
        rf"(?<![\w`$-]){re.escape(bin_name)}(?:\.exe)?\s+([a-z][\w-]*)",
        re.IGNORECASE,
    )
    for match in unquoted.finditer(combined):
        # Skip if this occurrence sits inside backticks: the preceding char
        # was already excluded for ` but inner spans can still match. Cheap
        # check: if the nearest backticks wrap it, skip.
        start = match.start()
        before = combined.rfind("`", 0, start)
        after = combined.find("`", start)
        if before != -1 and after != -1 and combined[before:after].count("`") % 2 == 1:
            continue
        cmd = match.group(1)
        if cmd in top_level or any(p[:1] == [cmd] for p in public):
            check.warn(
                f"unquoted `{bin_name} {cmd}` — quote live CLI invocations in backticks"
            )


# ----------------------------------------------------------------- package docs


def check_triggers(skills_dir: Path, router: str, check: Check) -> None:
    triggers = skills_dir / "references" / "triggers.md"
    if not triggers.is_file():
        check.fail("missing references/triggers.md (mechanical trigger preservation)")
        return
    phrases: list[str] = []
    for line in triggers.read_text(encoding="utf-8").splitlines():
        stripped = line.strip()
        if stripped.startswith("- "):
            phrases.append(stripped[2:].strip().strip('"').strip("'"))
    if not phrases:
        check.fail("references/triggers.md has no `- ` phrases")
        return
    first_screen = "\n".join(router.splitlines()[:FIRST_SCREEN_LINES])
    for phrase in phrases:
        if not phrase:
            continue
        if phrase not in first_screen:
            check.fail(
                f"trigger phrase {phrase!r} is missing from SKILL.md frontmatter + first {FIRST_SCREEN_LINES} lines"
            )


def check_package_docs(skills_dir: Path, router: str, version: object, check: Check) -> None:
    refs = skills_dir / "references"
    if not (refs / "get-started.md").is_file():
        check.fail("missing references/get-started.md")
    whats_new = refs / "whats-new.md"
    if not whats_new.is_file():
        check.fail("missing references/whats-new.md")
    elif isinstance(version, str) and version:
        heading = next(
            (line for line in whats_new.read_text(encoding="utf-8").splitlines() if line.lstrip().startswith("#")),
            None,
        )
        if heading is None:
            check.fail("references/whats-new.md has no heading line")
        elif not re.search(r"(?<![\w.])" + re.escape(version) + r"(?![\w]|\.\d)", heading):
            check.fail(f"references/whats-new.md first heading does not name version {version}")
    for literal in ROUTER_LITERALS:
        if literal not in router:
            check.fail(f"router SKILL.md is missing the literal `{literal}`")


def check_cold_read(skills_dir: Path, payload: dict, surface: Surface, check: Check) -> None:
    path = skills_dir / "references" / "cold-read.json"
    if not path.is_file():
        check.fail("missing references/cold-read.json")
        return
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (json.JSONDecodeError, UnicodeDecodeError) as exc:
        check.fail(f"cold-read.json is not JSON: {exc}")
        return
    if not isinstance(data, dict):
        check.fail("cold-read.json must be a JSON object")
        return
    if data.get("schema") != COLD_READ_SCHEMA:
        check.fail(f"cold-read.json schema is {data.get('schema')!r}, expected {COLD_READ_SCHEMA!r}")
    if not isinstance(data.get("plugin_id"), str) or not data["plugin_id"]:
        check.fail("cold-read.json plugin_id is missing")
    if data.get("version") != payload.get("version"):
        check.fail(
            f"cold-read.json version {data.get('version')!r} is not the dump version {payload.get('version')!r}"
        )
    if data.get("unclear") != []:
        check.fail(f"cold-read.json unclear is not empty: {data.get('unclear')!r}")
    capabilities = data.get("capabilities")
    if not isinstance(capabilities, list):
        check.fail("cold-read.json capabilities must be an array")
        return
    covered: set[tuple[str, ...]] = set()
    for item in capabilities:
        if (
            not isinstance(item, dict)
            or not isinstance(item.get("goal"), str)
            or not item["goal"].strip()
            or not isinstance(item.get("command"), list)
            or not item["command"]
            or not all(isinstance(part, str) and part for part in item["command"])
            or not isinstance(item.get("flags"), list)
            or not all(isinstance(flag, str) for flag in item["flags"])
        ):
            check.fail(f"cold-read.json capability must be {{goal, command: [..], flags: [..]}}: {item!r}")
            continue
        command = tuple(item["command"])
        joined = " ".join(command)
        if command not in surface.flags:
            check.fail(f"cold-read.json command `{joined}` is not a public command")
            continue
        covered.add(command)
        allowed = surface.allowed_flags(list(command))
        for flag in item["flags"]:
            if flag not in allowed:
                check.fail(f"cold-read.json flag {flag} does not exist for `{joined}`")
    for public in surface.paths:
        if tuple(public) not in covered:
            check.fail(f"cold-read.json does not cover public command `{' '.join(public)}`")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--bin", required=True, help="absolute path of the binary being published")
    parser.add_argument("--skills", required=True, help="skills/<product> directory")
    parser.add_argument(
        "--write-surface",
        action="store_true",
        help="write references/command-surface.md from the dump, then run all checks",
    )
    args = parser.parse_args(argv)

    bin_path = Path(args.bin)
    skills_dir = Path(args.skills).expanduser().resolve()
    if not skills_dir.is_dir():
        die(f"--skills is not a directory: {skills_dir}")

    check = Check()
    _skill_path, router, combined = load_markdown(skills_dir)
    payload = run_commands_json(bin_path)
    surface = validate_schema(payload, bin_path, check)
    dump_valid = not check.failures
    bin_name = str(payload.get("bin") or bin_path.stem)
    if dump_valid:
        if args.write_surface:
            target = skills_dir / "references" / SURFACE_NAME
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(render_surface(payload).encode("utf-8"))
        check_surface_file(skills_dir, payload, check)
    if surface.paths:
        direction_b(surface.paths, combined, check)
        direction_a(combined, router.splitlines(), bin_name, surface, check)
    check_triggers(skills_dir, router, check)
    check_package_docs(skills_dir, router, payload.get("version"), check)
    if dump_valid:
        check_cold_read(skills_dir, payload, surface, check)

    for warning in check.warnings:
        print(f"WARN  {warning}")
    if check.failures:
        for failure in check.failures:
            print(f"FAIL  {failure}")
        print(f"RESULT: FAIL ({len(check.failures)} failure(s), {len(check.warnings)} warning(s))")
        return 1
    print(
        f"ok    {bin_name} commands --json ({len(surface.paths)} public paths) matches {skills_dir}"
    )
    print("RESULT: PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
