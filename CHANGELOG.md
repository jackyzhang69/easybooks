## 0.5.22

- Transaction deletion support (`easybooks tx delete <id> [--force]`)
  via `DELETE /api/integrations/transactions/{id}`.

## 0.5.21

- Pair session with Jacky's assistant (`easybooks pair …`). Public
  `commands --json` lists the pair surface and still omits admin.
- Packed skill is a product conversation router. Binary resolution prefers
  `$EASYBOOKS_BIN`, then `~/.jackyzhang.app/plugins/easybooks/current`.

## 0.5.20

- First-session and update orientation briefs for the host agent. Shared
  Jacky auth and Tell-Jacky behavior is unchanged from 0.5.19.

## 0.5.19

- Runtime config lives at `~/.jackyzhang.app/easybooks/config.json`. Existing
  `~/.easybooks/config.json` is read once and migrated. Portal token stays in
  the shared user slot.

## 0.5.17

- Product HTTP exchanges `aud=eb` and sends the short-lived app JWT (memory
  only). Raw `jz_` is never a product Bearer. Re-exchange once on 401.

## 0.5.16

- Product Signals funnel: invoice send/mark, client create, rule create, and
  doctor now emit after the backend round-trip. Fail-open.

## 0.5.15

- Emit `posting_completed` after a successful write (Product Signals). The CLI
  posts `events:batch` on the calling thread so `process::exit` cannot kill the
  request.

## 0.5.13

- `whoami` forwards optional backend `email` as display metadata.

## 0.5.12

- Bump published client to 0.5.12.

## 0.5.11

- Publish latest main (admin feedback / auth hardening).

## 0.5.10

- Unified platform `user.json` connect; load-time migrate legacy token paths; host-agent file/plaintext token delivery rules.

# Changelog

## 0.5.9 — 2026-08-02

- Owner `jz_` is sent directly for product HTTP and Tell-Jacky (no required exchange).
- Aligns with portal one-connect auth governance.

## 0.5.8 — 2026-08-02

- Build fix: silence dead_code under release `-D warnings`.

## 0.5.7 — 2026-08-02

- Portal-first auth: `easybooks login --token-stdin` accepts owner `jz_` into
  `~/.jackyzhang.app/token/jz.json`; legacy `eb_live_` keys still work.
- Product HTTP exchanges `aud=eb` and sends the short-lived app JWT (memory only).
- Tell Jacky: `easybooks feedback create|status` → accountd `product_feedback`
  for plugin_id `easybooks`; new public skill `tell-jacky`.

All notable changes to the EasyBooks plugin (`easybooks`) are recorded here.

## 0.5.6 — 2026-07-27

- Isolated upgrade discovery to EasyBooks' own immutable `plugin-v*` releases;
  unrelated marketplace and product tags can no longer trigger false warnings.
- Added a tag-source/filter matrix covering valid releases, unrelated tags,
  prereleases, and malformed names.

## 0.5.5 — 2026-07-27

- Corrected every agent-facing `doctor` example so the top-level `--json` flag
  precedes the subcommand.
- Added a docs-to-parser command-contract matrix that rejects this argument-order
  regression across both skill mirrors and the installed verifier path.

## 0.5.0 — 2026-07-27

- Removed secret-bearing `login --token <value>` argv input. One-time login now
  uses bounded stdin and a non-echoing terminal prompt.
- Added atomic symlink-safe credential writes, mode-0700 directory and mode-0600
  file enforcement, default-production/URL-precedence coverage, and argv/output
  leak regression tests.
- Updated connection skills so the user enters the token locally; chat, agent
  tool calls, shell history, and generated token-bearing pipes are prohibited.
- Aligned plugin governance with the platform-vault current-session production
  gate and fixed the current Rust Clippy release gate.

## 0.1.0 — unreleased

Initial scaffold of the EasyBooks Codex/Claude Code plugin. Records invoices,
receipts, and Gmail-sourced finance data into EasyBooks through a bundled
`easybooks` CLI, which is the only boundary for EasyBooks reads and writes.

- **CLI (`cli/`)**: `easybooks` binary scaffold with the v1 command surface —
  setup/health (`login`, `whoami`, `doctor`), reads (`categories list`,
  `clients list`/`find`, `invoices list`), transaction recording
  (`income add`, `expense add`, `tx import-json`), invoices
  (`invoice create`, `invoice send`), and Gmail (`gmail record`, and
  `gmail sync` as a v1 stub). JSON-on-stdout output; integration key masked as
  `eb_***`; config persisted to `~/.easybooks/config.json` (mode `0600`).
- **Binary resolver**: `$EASYBOOKS_BIN` → `$CLAUDE_PLUGIN_ROOT/bin/<platform>/`
  → Codex cache → `command -v easybooks`.
- **Skills** (mirrored byte-identical in `.claude/skills/` and
  `.agents/skills/`): `connect-easybooks`, `easybooks-capabilities`
  (read-first router), `easybooks-record`, `easybooks-invoice`,
  `easybooks-gmail`.
- **Backend integration (host app `/Users/jacky/eb`, dev artifact)**: new
  `/api/integrations/*` endpoints (`whoami`, `ingest/transactions`,
  `ingest/invoice`, `categories`, `clients`, `invoices`) modeled on the existing
  audit→EasyBooks handler; idempotent on `(user_id, source_system, source_id)`.
  Invoice idempotency migration `009_invoice_external_sources.sql` added (not
  auto-applied). New env key names declared:
  `EASYBOOKS_INTEGRATION_API_KEY`, `EASYBOOKS_INTEGRATION_USER_ID`.
- **plugin-metadata**: `.claude-plugin/plugin.json`, `.codex-plugin/plugin.json`,
  `runtime-manifest.json`, and a marketplace-facing README.
- **CI / packaging**: `ci.yml`, `publish.yml`, and `scripts/` for local build,
  runtime-readiness verification, and skill-mirror sync.
- **Docs**: build contract (`docs/CONTRACT.md`, the single source of truth) and
  `docs/architecture.md`.
- **Governance**: PROD-default backend (`https://easybooks.jackyzhang.app`, the
  immicore eb-plugin via the eb frontend nginx `/api` proxy; test override
  `https://easybooks-test.jackyzhang.app`, LAN `http://192.168.1.69:8310`); any
  production write is approval-gated; secrets declared by name only (no values);
  backend deploys and migrations are not performed in this build.
