# easybooks — Codex / Claude Code plugin

Bookkeeping for self-employed Canadians. Wraps the **EasyBooks** Rust CLI in Codex / Claude Code skills so you can drop a receipt, paste an invoice, or scan your Gmail and have it recorded straight into EasyBooks.

## What it does

| Skill | Purpose |
|---|---|
| [`easybooks`](./skills/easybooks/SKILL.md) | The only discovery file. Connect, record, invoice, Gmail, and tell-jacky playbooks live under `skills/easybooks/references/`. |

## Core value

Drop a receipt or an invoice — or ask the agent to scan your connected Gmail for receipts and invoices — and it is recorded into EasyBooks as income or expenses, idempotently keyed so re-scans never double-record. You can also create and send invoices and resolve clients and categories by name.

## Agent contract

- **The bundled `easybooks` CLI is the only boundary for EasyBooks reads and writes.** No direct database access, no direct backend HTTP from the agent.
- The agent may read local text / image / PDF / email content itself to extract structured data, but the moment data is recorded, listed, or mutated it goes through `easybooks ...`.
- New transactions from files / email use: parse locally → `easybooks tx import-json --dry-run` → confirm → `easybooks tx import-json`.
- Invoices use: prepare JSON → `easybooks invoice create --dry-run` → confirm → `easybooks invoice create`, then optionally `easybooks invoice send <invoice_id>`.
- Runtime truth is `easybooks commands --json` plus the `easybooks` router skill. Never from memory of an older release.

## The only thing the user inputs

The user's durable platform **Portal token** (`jz_…`, always shown masked), shared by every official Jacky plugin and stored at `~/.jackyzhang.app/token/user.json`. It both authenticates and identifies the user — there is no owner id to provide. Retired `eb_live_` product keys are rejected by the CLI.

The token reaches `easybooks login --token-stdin [--base-url <url>]` over a non-echoing channel. The CLI writes the shared slot `~/.jackyzhang.app/token/user.json` (mode `0600`; `%USERPROFILE%\.jackyzhang.app\token\user.json` on Windows). Every other skill and every other official Jacky plugin reads that same slot — the token never enters argv, chat, shell history, or agent tool input; it is never asked for again and never echoed.

## Key safety (enforced by skill text)

- The key is **never** printed. Skills mask any value as `eb_***`.
- The key is **never** written into any file other than the CLI's own `config.json`.
- The key is **never** transmitted to anything other than the configured EasyBooks backend base URL.

## Dev vs production

The CLI defaults to the **PROD** backend (`https://easybooks.jackyzhang.app`) — the immicore Go eb-plugin reached via the eb frontend domain's nginx `/api` proxy. The legacy Node backend on `http://localhost:8080` is no longer the default. Override with `--base-url` / `$EASYBOOKS_API_URL` for **test** (`https://easybooks-test.jackyzhang.app`) or a **LAN** dev backend (e.g. `http://192.168.1.69:8310`). Because the default is production, mutations require the current-session authorization named by the platform-vault project card.

## Per-platform binary selection

`runtime-manifest.json` declares:

| Platform key | Binary path |
|---|---|
| `darwin-arm64` | `bin/darwin-arm64/easybooks` |
| `win32-x64` | `bin/win32-x64/easybooks.exe` |
| `linux-x64` | `bin/linux-x64/easybooks` |

Each skill resolves the binary in this order — first existing executable wins:

1. `$EASYBOOKS_BIN` (explicit override)
2. `$CLAUDE_PLUGIN_ROOT/bin/<platform>/easybooks`
3. Codex cache: `$HOME/.codex/plugins/cache/jacky-plugins/easybooks/<highest-version>/bin/<platform>/easybooks`
4. `command -v easybooks` (manual PATH install)

The public bundle currently supports `darwin-arm64`, `win32-x64` and `linux-x64`. Other hosts require an explicit trusted binary override or PATH installation.

## Install

See the repo-root [README](../README.md) for `codex plugin marketplace add` / `codex plugin install` commands.

## What this plugin is NOT

- Not an MCP server — every skill shells out to `easybooks <subcommand>`.
- Not a re-implementation of EasyBooks logic.
- Not a place to store credentials in source.
- No lazy / on-demand asset downloads — the single `easybooks` binary is the only runtime artifact.
