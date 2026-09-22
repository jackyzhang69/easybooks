# easybooks

Record your bookkeeping straight from a conversation. Drop a receipt, paste an
invoice, or point at your inbox — the agent reads it, and **EasyBooks** files it
as income, an expense, or an invoice. No spreadsheets, no double entry, no
copy-paste between your email and your books.

This is the container repo for the **EasyBooks** Codex/Claude Code plugin
(`easybooks`). The plugin wraps a bundled `easybooks` CLI; it is not a
separate re-implementation of EasyBooks business logic. The agent parses
documents locally, then records everything through the CLI.

## What you get

- Drop a receipt or PDF → it lands as an expense in EasyBooks.
- Paste invoice text or an image → it becomes a recorded invoice you can send.
- "Log a $120 software expense for May 1" → done, categorized, in your books.
- "Scan my Gmail for receipts" → the agent reads candidate emails and records
  them once each, with no double-counting on re-scan.

Everything is idempotent: the same source document recorded twice does not
create a duplicate.

## Components

- **`cli/`** — the bundled `easybooks` CLI. This is the only supported boundary
  for EasyBooks reads and writes (record transactions, create/send invoices,
  list categories/clients/invoices, health checks).
- **`.claude/skills/`** — Claude Code agent skill docs.
- **`.agents/skills/`** — Codex/agent mirror of the same skill docs, kept
  byte-identical with `.claude/skills/`.
- **`plugin-metadata/`** — marketplace manifests, runtime manifest, and
  marketplace-facing README.
- **`docs/`** — the build contract (`CONTRACT.md`, the single source of truth)
  and `architecture.md`.

(The EasyBooks backend integration endpoints live in the host app repo
`/Users/jacky/eb`, not here. See [`docs/architecture.md`](docs/architecture.md).)

## Agent contract — the CLI is the only boundary

The whole reason this plugin exists: **the bundled `easybooks` CLI is the only
boundary for EasyBooks system reads and writes.**

- Agents may parse files locally (text, image, PDF, email) to extract structured
  data.
- The moment data is recorded, listed, or mutated, it goes through
  `easybooks ...`.
- No direct Supabase writes from the agent. No direct backend HTTP from the
  agent. The CLI is the only thing that talks to the EasyBooks backend.
- `easybooks-capabilities` is the session entrypoint: it carries the
  intent → command router, the binary resolver, and the file-import decision
  tree. The agent reads it first every session, then routes to
  `easybooks-record`, `easybooks-invoice`, or `easybooks-gmail`.

The file-import path is always: parse locally → `easybooks tx import-json
--dry-run` → confirm the resolved rows → `easybooks tx import-json`.

## Build

```bash
cargo build --release --target aarch64-apple-darwin
```

Local packaging copies the built binary into `bin/darwin-arm64/easybooks`
(see `scripts/build-local.sh`). The published plugin ships per-platform
binaries selected via `runtime-manifest.json`.

## Connect flow

The only thing you input is your platform Portal token (`jz_…`, masked
everywhere). It is the **same token every official Jacky plugin uses** — if you have
already connected FormBro, AnyChat, AnyPDF, or AnyDoc, EasyBooks reuses it and you
connect nothing. Retired `eb_live_` product keys are rejected by the CLI. The token
both authenticates and identifies you, so there is no
owner id to provide.

1. Run the `connect-easybooks` skill. It captures the platform Portal token and
   the backend base URL — and skips straight to step 3 if you already connected
   any other official Jacky plugin.
2. The token is fed to `easybooks login --token-stdin [--base-url <url>]` over a
   non-echoing channel, never on the command line. It writes the shared slot
   `~/.jackyzhang.app/token/user.json` (mode `0600`).
3. It confirms the connection with `easybooks whoami` (which echoes your user id
   and scope) and `easybooks doctor`.

Every other skill reads from that config. The key never enters argv, shell history,
chat, or agent tool input; it is never asked for again, never echoed, and never
stored anywhere except that single config file.

## Governance

EasyBooks is governed under the platform-vault protocol. This build is a
**development artifact** and follows the contract's governance section
(`docs/CONTRACT.md` §6). Read this before pointing the CLI anywhere real.

- **Production by default.** The CLI defaults to the PROD backend
  (`https://easybooks.jackyzhang.app`) — the immicore Go eb-plugin reached via
  the eb frontend domain's nginx `/api` proxy. The legacy Node backend on
  `http://localhost:8080` is no longer the default. Override with `--base-url` /
  `$EASYBOOKS_API_URL` for **test** (`https://easybooks-test.jackyzhang.app`) or
  a **LAN** dev backend (e.g. `http://192.168.1.69:8310`). Because the default is
  production, any write is gated by the current platform-vault project card and
  requires the named explicit current-session authorization; the
  skills warn before any production write.
- **Secrets are names, not values.** The user's API key is a secret. It is
  never printed or logged and lives only in `~/.jackyzhang.app/token/user.json` (CLI
  side). Keys are minted per-user in the EasyBooks web app and carry a scope
  (`read` / `read_write`); a user rotates or revokes their own key there. No
  secret values are generated or committed by this build.
- **No production deploys, no auto-applied migrations.** The backend integration
  changes and the invoice-source migration
  (`009_invoice_external_sources.sql`) are added as dev artifacts only.
  Deploying the backend and applying migrations to any database are separate,
  approval-gated steps and are **not** performed by this build.

## License

MIT — see [LICENSE](LICENSE). Copyright (c) 2026 Jacky Zhang.
