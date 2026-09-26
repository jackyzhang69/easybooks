# EasyBooks Plugin — Authoritative Build Contract (SSOT)

This file is the single source of truth for the EasyBooks Codex/Claude Code plugin.
Every component (Rust CLI, backend integration endpoints, skills, manifests, CI, docs)
MUST conform to the names, shapes, and rules defined here. If a component needs to
deviate, update THIS file first so all components stay in lockstep.

Reference implementation to mirror for *structure and idioms* (not business logic):
`/Users/jacky/formbro-plugin` — especially `cli/src/{main,config,client,output}.rs`,
`cli/src/commands/*`, `.claude/skills/*`, `plugin-metadata/*`, `.github/workflows/*`.

The host app being integrated: `/Users/jacky/eb` (EasyBooks). A self-employed finance
app: React frontend writes directly to Supabase (anon key + Auth/RLS); a thin Express
backend (service-role key) exposes a few endpoints, including ONE existing integration
endpoint pattern we are extending: `POST /api/integrations/audit/stripe-payouts`
(see `eb/backend/src/index.ts`), which idempotently upserts income/expense into
`eb_transactions` keyed on `(user_id, source_system, source_id)`.

---

## 0. Naming & identity (do not vary)

- Plugin name (marketplace + manifests): `easybooks`
- Display name: `EasyBooks`
- CLI binary name: `easybooks`
- Rust crate: package `easybooks-cli`, `[[bin]] name = "easybooks"`, lib `easybooks_cli`
- Config file: `~/.jackyzhang.app/easybooks/config.json` (legacy `~/.easybooks/config.json` is migrated on read)
- Binary override env: `$EASYBOOKS_BIN`
- Version: `0.1.0` (workspace-inherited)
- Marketplace repo (published plugin): `jackyzhang69/plugins` (same marketplace as formbro)
- Source/container repo: `https://github.com/jackyzhang69/easybooks`

### Binary resolver (used by every skill; mirror formbro-capabilities §B)
Resolution order — first existing executable wins:
1. `$EASYBOOKS_BIN` (explicit override)
2. `$CLAUDE_PLUGIN_ROOT/bin/<platform>/easybooks` (Claude Code sets `CLAUDE_PLUGIN_ROOT`)
3. Codex cache: `$HOME/.codex/plugins/cache/jacky-plugins/easybooks/<highest-version>/bin/<platform>/easybooks`
4. `command -v easybooks` (manual PATH install)

The public bundle currently supports `darwin-arm64` and `win32-x64`; other hosts require an explicit trusted binary override or PATH installation.

---

## 1. Boundary rule (the whole reason the plugin exists)

**The bundled `easybooks` CLI is the ONLY boundary for EasyBooks system writes and reads.**
- Agents may parse files locally (text, image, PDF, email) to extract structured data.
- The moment data is recorded, listed, or mutated, it goes through `easybooks ...`.
- No direct Supabase writes from the agent. No direct backend HTTP from the agent.
- The CLI talks to the EasyBooks backend integration endpoints (§3) over HTTP.

---

## 2. CLI command surface (exact)

Global behavior:
- Output is JSON on stdout (machine-first), one JSON object per invocation. Mirror
  formbro `output.rs`: success → JSON object; errors → non-zero exit + JSON `{"error": "..."}`
  on stderr.
- Global flags: `--json` (default on; accepted for parity), `--base-url <url>` (override config),
  `--quiet`.
- The user API key is NEVER printed. Mask as `eb_***` anywhere it would appear.

Commands:

### Setup / health
- `easybooks login --token-stdin [--base-url <url>]`
  Persists `{ base_url }` to `~/.jackyzhang.app/easybooks/config.json` (mode 0600)
  and the portal `jz_` to the shared `~/.jackyzhang.app/token/user.json` slot.
  Token input is `--token-stdin` only; never argv. `--base-url`
  default `https://easybooks.jackyzhang.app` (PROD, immicore-served eb-plugin via the
  eb frontend nginx `/api` proxy). Override for test:
  `https://easybooks-test.jackyzhang.app`; for LAN: `http://192.168.1.69:8310`.
  Output: `{"status":"ok","path":".../easybooks/config.json","base_url":"...","api_key_masked":"jz_***"}`.
- `easybooks whoami`
  Calls `GET /api/integrations/whoami` (the key identifies the user). Output:
  `{ base_url, user_id, email?, scope, api_key_masked }` (user_id, optional email, and scope come from the whoami response).
- `easybooks doctor [--no-fetch] [--check-upgrade]`
  Local config check + backend round-trip + version. `--no-fetch` = pure local read (no network).
  `--check-upgrade` = one GitHub Tags API call (non-fatal). Output shape (mirror formbro doctor):
  `{ "binary_version":"0.1.0", "config":{"present":true,"path":"...","base_url":"..."},
     "backend":{"reachable":true,"status":"ok"},
     "cache":{"location":"...|not_in_cache","stale":false,"version":"...","latest_available":"..."},
     "upgrade":{"checked":false,"upgrade_available":false} }`

### Reads (so the agent can resolve names → ids; never guess ids)
- `easybooks categories list [--type income|expense]` → `GET /api/integrations/categories`
- `easybooks clients list` / `easybooks clients find --query <q>` → `GET /api/integrations/clients`
- `easybooks invoices list [--status <s>]` → `GET /api/integrations/invoices`

### Record transactions (income / expense) — the core "drop a receipt/invoice" path
- `easybooks income add --amount <decimal> --description <text> --date <YYYY-MM-DD>
     [--category <name>] [--classification business|personal] [--source-system <s>]
     [--source-id <id>] [--notes <text>] [--dry-run]`
- `easybooks expense add ...same flags...`
  Both wrap a single-entry call to `POST /api/integrations/ingest/transactions`.
  `--dry-run` validates + echoes the resolved row without writing.
- `easybooks tx import-json --json '<json>' [--dry-run]`
  Batch ingest. `<json>` is `{ "source_system":"...", "entries":[ <Entry>... ] }`
  where `<Entry>` = `{ "type":"income|expense", "amount_cents":int, "description":str, "date":"YYYY-MM-DD",
  "category_name?":str, "classification?":"business|personal", "source_type?":str, "source_id":str,
  "source_payload?":obj }`. Idempotent (§3). This is the primary boundary for "agent parsed a
  document/email → record it". Output: `{ "created":n, "existing":n, "skipped":n, "processed":n }`.

### Invoices
- `easybooks invoice create --json '<json>' [--dry-run]` → `POST /api/integrations/ingest/invoice`
  `<json>` = `{ "client": {"name":str,"email?":str,"address?":str,"phone?":str} | {"client_id":uuid},
  "issue_date":"YYYY-MM-DD", "due_date":"YYYY-MM-DD", "tax_rate?":number,
  "items":[{"description":str,"quantity":number,"unit_price":number}], "notes?":str,
  "payment_details?":str, "source_id?":str }`. Resolves/creates client; computes subtotal/tax/total
  server-side; idempotent on `(user_id, source_id)` when `source_id` given, else `(user_id, invoice_number)`.
  Output: `{ "invoice_id":uuid, "invoice_number":str, "total":number, "created":bool }`.
- `easybooks invoice send <invoice_id>` → `POST /api/integrations/invoice/:id/send`
  (sends invoice/receipt email). This integration route authenticates with the user's API key
  (`Authorization: Bearer <api_key>`, scope `read_write`; §3), then reuses the existing legacy
  send/email logic. We do NOT call the legacy `POST /api/invoices/:id/send` directly because that
  route authenticates via the frontend `x-user-id` header, which the CLI does not send. Output
  passthrough.

### Gmail
- v1 (this build): Gmail reading is done by the agent via the connected Gmail MCP. The CLI's role
  is to RECORD what the agent extracted. Provide:
  - `easybooks gmail record --json '<json>' [--dry-run]` — alias of `tx import-json` with
    `source_system` defaulted to `gmail`; requires each entry's `source_id` to be the Gmail
    message id (guarantees idempotency / no double-recording on re-scan).
  - `easybooks gmail sync` — v1 STUB: returns `{"status":"not_implemented_v1","hint":"In v1, read Gmail
    via the Gmail MCP and record with `easybooks gmail record`. Native OAuth sync ships in v2."}`
- v2 (documented, not built now): native Gmail OAuth in the CLI (`gmail sync` pulls candidate
  receipts/invoices headless for cron). Leave a clear seam (a `gmail` command module) and a TODO.

---

## 3. Backend integration endpoints (add to `/Users/jacky/eb/backend`)

Add a new router module (e.g. `eb/backend/src/integrations.ts`) wired into `index.ts`, OR add the
routes directly in `index.ts` next to the existing audit endpoint — match the existing code style
(plain Express handlers, `supabase` service-role client, `try/catch` with `console.error`).

Auth: per-user API keys. Users generate a personal key in the EasyBooks web app
(Settings → API Keys), scope `read` or `read_write`. The key is `eb_live_...`. Every
request carries header `Authorization: Bearer <api_key>`. The backend validates the key,
derives the owning `user_id` and `scope` from it (the key IS the identity — there is no
separate owner id and no shared key). Write endpoints (`ingest/transactions`,
`ingest/invoice`, `invoice/:id/send`) require scope `read_write`; reads require `read`.
Invalid/missing key → 401; insufficient scope → 403.

DO NOT print/log key values. DO NOT deploy to production (governance §6).

### Endpoints
- `GET  /api/integrations/whoami` → `{ ok:true, user_id, scope, source:"easybooks-integration" }`
  (`user_id` + `scope` derived from the Bearer key). Requires scope `read`.
- `POST /api/integrations/ingest/transactions` (requires scope `read_write`)
  Body: `{ source_system, entries:[<Entry>] }` (Entry per §2; the owning user comes from the key).
  Behavior: mirror the existing `/api/integrations/audit/stripe-payouts` handler — resolve
  `category_id` via the existing
  `resolveCategoryId(userId, type, category_name)` helper; skip zero-amount; build rows with
  `status:'completed'`, `classification` (default `'business'` when omitted), `source_system`,
  `source_type`, `source_id`, `source_payload`; `upsert(rows, { onConflict: 'user_id,source_system,source_id' })`.
  Return `{ status:'ok', created, existing, skipped, processed }`.
- `POST /api/integrations/ingest/invoice` (requires scope `read_write`)
  Resolve/create client in `eb_clients` (by client_id, else by (user_id,email|name)); insert
  `eb_invoices` (generate invoice_number if absent — reuse the same scheme as the frontend,
  `INV-` prefix; check `frontend/src/lib/utils.ts:generateInvoiceNumber`) + `eb_invoice_items`.
  Compute subtotal/tax_amount/total server-side. Idempotency: add migration
  `eb/supabase/migrations/009_invoice_external_sources.sql` adding `source_system,source_id,source_payload`
  + unique index `(user_id, source_system, source_id)` to `eb_invoices`, mirroring migration 006; when
  `source_id` provided, upsert/short-circuit on it. (Migrations are NOT auto-applied — note in docs.)
  Return `{ invoice_id, invoice_number, total, created }`.
- `POST /api/integrations/invoice/:id/send` (requires scope `read_write`)
  API-key-authenticated wrapper around the existing invoice/receipt email send. Auth:
  `Authorization: Bearer <api_key>` (NOT the legacy `x-user-id` header); the owning `user_id`
  comes from the key. Looks the invoice up by `(id, user_id)` and delegates to the shared
  `sendInvoiceById(id, userId)` helper (extracted from the legacy `POST /api/invoices/:id/send`
  handler in `index.ts`, which now also delegates to it). Output passes through the helper result
  (e.g. `{ success:true, message:"Invoice sent successfully", emailId }`, or the matching 4xx/5xx
  error body for not-found / no-client-email / send-failure).
- Reads (service-role, filtered by the key's user id; require scope `read`):
  - `GET /api/integrations/categories?type=income|expense`
  - `GET /api/integrations/clients?query=<q>`
  - `GET /api/integrations/invoices?status=<s>`

Note: `POST /api/integrations/audit/stripe-payouts` is a SEPARATE, pre-existing audit endpoint
with its own auth; it is unchanged by this per-user-key model.

---

## 4. Skills (mirror in BOTH `.claude/skills/` and `.agents/skills/`, kept byte-identical)

Each skill is `<name>/SKILL.md` with frontmatter `name`, `description`, `when_to_use`.
Skill set:
- `connect-easybooks` — one-time setup using the shared Portal `jz_` (same contract as every official
  plugin). Host agent pipes a token file: `printf %s "$(cat -- "$TOKEN_FILE")" | easybooks login --token-stdin
  [--base-url <url>]`. File preferred; chat paste allowed with one warning; never tell the human to use
  a terminal; never `--token` argv. Then `whoami`/`doctor`. Never log the key. Tells agent to load
  `easybooks-capabilities` next.
- `easybooks-capabilities` — READ-FIRST router. Top-20-line intent→command table. §B binary resolver
  (adapted to `easybooks`/`$EASYBOOKS_BIN`/`easybooks` cache path). Non-negotiable operating rules
  (CLI-only boundary; local parse → CLI record). The decision tree for "user dropped a file / pasted
  invoice text / image / PDF" → parse locally → `tx import-json --dry-run` → confirm → `tx import-json`.
- `easybooks-record` — income/expense + document/receipt ingestion. The file-import decision tree
  (Excel/CSV/PDF/image/email/text → parse to Entry JSON → dry-run → record). Idempotency guidance.
- `easybooks-invoice` — create + send invoices; client resolution; dry-run before create.
- `easybooks-gmail` — v1 flow: read candidate receipts/invoices via the Gmail MCP, extract to Entry
  JSON, record with `easybooks gmail record` (source_id = Gmail message id → no double-record). Document
  the v2 native-OAuth path as "coming". Include a copy-pasteable Gmail search query for receipts/invoices.

Routing principle: `easybooks-capabilities` is loaded first every session and points to the others.

---

## 5. plugin-metadata

- `.claude-plugin/plugin.json` — name `easybooks`, version, author, homepage, repository, license,
  keywords (`easybooks`, `bookkeeping`, `invoices`, `self-employed`, `canada`), description.
- `.codex-plugin/plugin.json` — superset with `skills: "./skills/"`, `min_supported_version`, and an
  `interface` block (displayName `EasyBooks`, shortDescription, longDescription, category `Productivity`,
  capabilities `["Read","Write"]`, defaultPrompt examples like:
  "Record this receipt into EasyBooks", "Log a $120 expense for software on 2026-05-01",
  "Create an invoice for <client> for <items>", "Scan my Gmail for receipts and record them").
- `runtime-manifest.json` — declares only the currently bundled `darwin-arm64` and `win32-x64` binaries. No lazy assets (unlike FormBro — no PDF/webform runtime).
- `README.md` (marketplace-facing) describing the plugin + the connect flow.

Note: the published plugin's `skills/` is populated from `.claude/skills` (or `.agents/skills`) by the
release packaging; for this repo, keep skills authored under `.claude/skills` + `.agents/skills` and have
CI/packaging copy them into the published bundle. Document the chosen mechanism in scripts.

---

## 6. Governance (platform-vault) — REQUIRED, surface to user

- EasyBooks is NOT a normalized vault project; the closest precedent is the audit→easybooks integration,
  whose **production** writes/deploys require the explicit current-session authorization named by the current platform-vault project card.
- The CLI **defaults to the PROD backend** (`https://easybooks.jackyzhang.app`), which is the immicore
  Go eb-plugin reached via the eb frontend domain's nginx `/api` proxy (`/api/integrations/*`). The legacy
  Node backend on `http://localhost:8080` is no longer the default. Override with `--base-url` /
  `$EASYBOOKS_API_URL` for **test** (`https://easybooks-test.jackyzhang.app`, served by immicore-test) or
  **LAN** (`http://192.168.1.69:8310`). Because the default is production, any **write** command (record,
  import, invoice create/send) is a production write: it is gated by the current platform-vault project card, and the
  skills must warn the user before any production write.
- The user's Portal token is a secret: never printed/logged; lives only in `~/.jackyzhang.app/token/user.json`. Runtime config is `~/.jackyzhang.app/easybooks/config.json`.
  Keys are minted per-user in the EasyBooks web app and carry a scope (`read` / `read_write`); a user
  rotates/revokes their own key there. We do NOT generate secret values in this build.
- DO NOT deploy the backend change to production and DO NOT apply migrations to any DB in this build.
  Backend code + migration file are added as dev artifacts; applying/deploying is a separate, gated step.

---

## 7. CI / packaging (full formbro-level, but Rust-only & simpler)

- `.github/workflows/ci.yml` — on PR/push to main: `cargo build --release --target aarch64-apple-darwin`,
  `cargo clippy --release -- -D warnings`, `cargo test --release`. Select any free Mac runner by
  capability labels only: `runs-on: [self-hosted, macOS, ARM64]`. Do not pin a named runner
  (`easybooks-plugin-signing`).
- `.github/workflows/publish.yml` — on immutable `plugin-v*` tag, verify exact version/source,
  build `aarch64-apple-darwin` + `x86_64-pc-windows-gnu`, require macOS signing/notarization,
  and upload one signed stage plus checksum to the source-repository Release. It never pushes the
  marketplace. Same capability labels as CI. Keep the deploy concurrency group
  `easybooks-plugin-${{ github.ref }}`.
- Marketplace publication consumes the verified source Release, assembles all pending plugin
  packages, reviews the combined tree, and performs one normal fast-forward marketplace push.
- `scripts/build-local.sh` — `cargo build --release --target aarch64-apple-darwin` then copy to
  `bin/darwin-arm64/easybooks`; optional codesign (env `CODESIGN_IDENTITY`).
- `scripts/verify-runtime-readiness.sh` — JSON-emitting readiness gate: required files present
  (manifests, skills, resolved binary executable). Model on formbro's.
- `scripts/sync-skills.sh` — assert `.claude/skills` and `.agents/skills` are byte-identical (or sync one
  to the other); fail CI on drift.

---

## 8. Repo file tree (target)

```
easybooks-plugin/
  Cargo.toml                 (workspace — already written)
  .cargo/config.toml         (already written)
  .gitignore                 (already written)
  rust-toolchain.toml        (optional; stable)
  README.md
  CHANGELOG.md
  LICENSE                    (MIT, Jacky Zhang)
  cli/
    Cargo.toml
    src/{main.rs,lib.rs,config.rs,client.rs,output.rs,doctor.rs}
    src/commands/{mod.rs,setup.rs,read.rs,transactions.rs,invoices.rs,gmail.rs}
    src/bootstrap/{mod.rs,resolve.rs}   (binary/version resolution; no lazy assets)
    tests/*.rs                          (assert_cmd + mockito for endpoint contracts)
  .claude/skills/<5 skills>/SKILL.md
  .agents/skills/<5 skills>/SKILL.md    (byte-identical mirror)
  plugin-metadata/{.claude-plugin/plugin.json,.codex-plugin/plugin.json,runtime-manifest.json,README.md}
  .github/workflows/{ci.yml,publish.yml}
  scripts/{build-local.sh,verify-runtime-readiness.sh,sync-skills.sh}
  docs/{CONTRACT.md (this file),architecture.md}
  bin/<platform>/                       (populated by build/CI; gitignored)
```

(Backend changes live in `/Users/jacky/eb`, NOT in this repo.)
