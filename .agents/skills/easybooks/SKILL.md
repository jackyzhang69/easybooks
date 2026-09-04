---
name: easybooks
description: >-
  READ THIS FIRST for EasyBooks. Bookkeeping for self-employed Canadians:
  record this receipt, log a $X expense/income, I dropped a file / invoice /
  screenshot, here's a receipt / supplier invoice / screenshot / scanned PDF —
  record it, import this spreadsheet / bank statement / CSV of expenses,
  what categories do I have, create an invoice, send invoice X, bill a client,
  scan my Gmail for receipts, find purchase receipts in my email, import my
  email receipts, go through my Gmail and log the expenses, list my
  categories/clients/invoices, connect EasyBooks, log in / save my jz_
  platform token, tell Jacky. One discovery file; playbooks in references/.
  Ask the live CLI (`easybooks commands --json`), never from memory of an
  older release.
when_to_use: |-
  Load on plugin start; reload whenever a user asks anything EasyBooks-related.
  Trigger phrases: "record this receipt", "log a $X expense/income",
  "I dropped a file / invoice / screenshot", "create an invoice",
  "send invoice X", "scan my Gmail for receipts",
  "list my categories/clients/invoices",
  "is EasyBooks healthy / which backend am I on", "connect EasyBooks",
  "connect to easybooks / set up easybooks",
  "log in to easybooks / save my jz_ platform token",
  "configure the easybooks plugin / use this key + base url",
  "scan my Gmail / inbox for receipts / invoices and record them",
  "find purchase receipts in my email and add them to EasyBooks",
  "import my email receipts into the books",
  "go through my Gmail and log the expenses",
  "create / draft / make an invoice for <client> for <items>",
  "bill <client> $X for <work>",
  "send invoice <id> / email this invoice / email the receipt",
  "list my invoices / which invoices are unpaid / find client <name>",
  "log / record a $X expense for <thing> on <date>",
  "record this income / payment I received",
  "here's a receipt / supplier invoice / screenshot / scanned PDF — record it",
  "import this spreadsheet / bank statement / CSV of expenses",
  "what categories do I have", "tell Jacky".
---

# EasyBooks plugin — agent consumption contract

## Talk to the human (mandatory)

Speak in everyday words. Say what you are doing and what happens next. Do not
lead with binary paths, `--help`, or raw JSON. Keep CLI and JSON between
tools. Never show credentials, bank data, or receipt contents in chat unless
the user asked for that detail. Run the intended ordinary command first; when
stdout is `jz.plugin.envelope.v1`, follow its exact `status` and
`continue_args` until the product reports `ready`, then resume the sealed
request. Never assign doctor homework when an envelope already names the route.

## Live CLI surface (fail-closed)

Ask the live CLI, never from memory of an older release:

```bash
"$EASYBOOKS_BIN" commands --json
```

Playbooks: [connect](references/connect.md), [record](references/record.md),
[invoice](references/invoice.md), [gmail](references/gmail.md),
[tell-jacky](references/tell-jacky.md).

## Shared platform token (host agent — mandatory)

- Canonical durable user credential: `~/.jackyzhang.app/token/user.json` (`jz_` only).
- **One connect for the whole platform.** If this file already exists from any other official plugin, do **not** ask the human to log in again.
- On load, if `user.json` is missing but a legacy on-disk `jz_` exists (`token/user.json`, product-local config token fields), **migrate immediately** by running this product's CLI `whoami`/`doctor`/any authenticated command (load-time migrate is built in). User must feel no re-paste.
- Only when no migratable `jz_` exists: complete connect once using the token-delivery rules below. After that, every registered plugin reuses the same slot.
- Never print or log the raw token. Prefer masked doctor/whoami output.
- Do not create product-local durable token files.

## Token delivery to the host agent (connect) — LOCKED 2026-08-14

The host agent performs connect **for** the human. **Never** tell the human to open a terminal and run login commands themselves.

Accept input in this order:

1. **File containing the token (preferred).** If the human provides a filesystem path (e.g. `~/Desktop/jacky-token.txt`) or an attached/readable file whose contents are a single `jz_…` value (optional surrounding whitespace/newline only):
   - Read the file in the agent tool channel.
   - Pipe the token to the product CLI via stdin only: `login --token-stdin` (or the product's equivalent).
   - Do **not** put the token on argv, in chat echo, in logs, or in screenshots.
   - Confirm success with masked doctor/whoami only.

2. **Plaintext token in chat (allowed, discouraged).** If the human pastes a raw `jz_…` into the conversation:
   - **Warn once** in plain language: prefer a local file path next time so the secret is not retained in chat history; do **not** refuse.
   - Proceed immediately: feed that value to `login --token-stdin` via a non-echoing agent-side pipe (temp file mode 0600 deleted after use is OK; never `echo TOKEN |` in a way that lands in shell history if avoidable).
   - Do not repeat the raw token back in the reply.

3. **No token yet.** Ask the human to send either a **file path** (best) or paste the token. Still do not ask them to run terminal commands.

Hard rules:

- `--token <value>` / argv secrets remain **forbidden** for the CLI.
- Agent may read a user-supplied path and stdin-feed the CLI; that is the supported file path.
- After any successful connect, other plugins must not re-prompt when `user.json` is present.


# EasyBooks plugin — agent consumption contract

**Read this once on plugin load and reload it whenever a user asks anything EasyBooks-related.** It tells you which skill / subcommand to call for each user intent, what to never guess, and where the production safety gate is.

EasyBooks is a self-employed (Canadian) finance app: income/expense transactions, categories, clients, and invoices. This plugin lets an agent record bookkeeping through one bundled CLI instead of touching the database or backend directly.

## 0. Non-negotiable operating rules

1. **All EasyBooks system operations go through the bundled CLI.** Reads (categories / clients / invoices), recording income/expense, importing parsed documents, creating invoices, and sending invoices must use `<easybooks> ...`. Do **not** call EasyBooks backend endpoints directly and do **not** write to Supabase / any database directly. The CLI is the only boundary.
2. **Local file parsing is allowed only before the CLI boundary.** The agent may read Excel, CSV, PDF, image (receipt photo / scan), email, or plain text locally to extract facts. The moment data is recorded, listed, or mutated, call the CLI.
3. **Never guess ids.** Resolve category names and client names to ids by recording with names (the backend resolves them) or by listing first (`categories list`, `clients find`, `invoices list`). Do not invent a `category_id` or `client_id`.
4. **Idempotency is mandatory for any recorded document.** Every parsed receipt / invoice / email row carries a stable `source_id`. Re-running the same import must not double-record. For Gmail, `source_id` is the Gmail message id (see `references/gmail.md`).
5. **Production is the default; writes are gated.** The CLI defaults to the PROD backend (`https://easybooks.jackyzhang.app`, the immicore eb-plugin via the eb frontend nginx `/api` proxy). Override to test (`https://easybooks-test.jackyzhang.app`) or LAN (`http://192.168.1.69:8310`) via `--base-url`. Follow the current platform-vault project card and require its current-session authorization before a production mutation — see §G.

## Agent quick router — TOP 20 LINES (read this first)

User said this → call this exact command (binary resolution: §B; file-drop decision tree: §C; full per-skill detail in the linked skill):

| User intent | Command (one-hop preferred) | Skill |
|---|---|---|
| "record / log a $X **expense** for `<thing>` on `<date>`" | `easybooks expense add --amount <d> --description "<t>" --date <YYYY-MM-DD> [--category <name>] [--classification business\|personal]` | references/record.md |
| "record / log a $X **income** / payment received on `<date>`" | `easybooks income add --amount <d> --description "<t>" --date <YYYY-MM-DD> [--category <name>]` | references/record.md |
| "here's a **receipt / invoice file** (PDF / image / Excel / CSV / email / text) — record it" | parse locally → build Entry JSON (§2) → `easybooks tx import-json --json '<json>' --dry-run` → show user → rerun without `--dry-run` | references/record.md |
| "record **several** transactions / a statement / a spreadsheet of expenses" | parse locally → batch Entry JSON → `easybooks tx import-json --json '<json>' --dry-run` → confirm → rerun | references/record.md |
| "this is **personal / mixed**, not business" / "**fix the classification**" | `easybooks tx reclassify <id> --class business\|mixed\|personal [--learn]` (`--learn` remembers the sender) | references/record.md |
| "**attach** the receipt / PDF to this transaction" | `easybooks tx attach-receipt <id> --file <path>` (reads locally, <=10MB, base64; prints `receipt_url`) | references/record.md |
| "**create an invoice** for `<client>` for `<items>`" | resolve client → build invoice JSON (§2) → `easybooks invoice create --json '<json>' --dry-run` → confirm → rerun | references/invoice.md |
| "**send** invoice `<id>` / email the invoice/receipt" | `easybooks invoice send <invoice_id>` (CONFIRM first — it emails the client) | references/invoice.md |
| "**scan my Gmail** for receipts / invoices and record them" | read candidates via Gmail MCP → extract to Entry JSON with `source_id` = Gmail message id → `easybooks gmail record --json '<json>' --dry-run` → confirm → rerun | references/gmail.md |
| "**find / search** a transaction by type / category / date / amount / text" | `easybooks tx list [--type income\|expense] [--classification business\|mixed\|personal\|unclassified] [--review needs_review\|reviewed] [--from YYYY-MM-DD] [--to YYYY-MM-DD] [--query <q>] [--limit n]` | references/record.md |
| "**confirm** a transaction (clear needs-review without changing classification)" | `easybooks tx confirm <id>` | references/record.md |
| "**edit** a transaction (amount / date / description / category / notes)" | `easybooks tx update <id> [--amount <d>] [--date YYYY-MM-DD] [--description "<t>"] [--category <name>] [--notes "<t>"] [--dry-run]` | references/record.md |
| "view receipt **URL** for a transaction" | `easybooks tx receipt-url <id>` (returns signed URL) | references/record.md |
| "**dashboard** summary (income / expenses / net / outstanding / tax estimate)" | `easybooks dashboard [--year <YYYY>]` | references/record.md |
| "list my **categories**" | `easybooks categories list [--type income\|expense]` | references/record.md |
| "**manage categories** (create / view)" | `easybooks categories create --name <n> --type income\|expense [--tax-deductible]` | references/record.md |
| "**manage clients** (list / create / update / delete)" | `easybooks clients list`, `easybooks clients create --name <n> [--email --phone --address --notes]`, `easybooks clients update <id> ...`, `easybooks clients delete <id>` | references/invoice.md |
| "get **invoice details**" | `easybooks invoice get <id>` | references/invoice.md |
| "**mark invoice** paid / unpaid" | `easybooks invoice mark <id> --status paid\|unpaid` | references/invoice.md |
| "**download invoice** PDF" | `easybooks invoice pdf <id> [--out <path>]` | references/invoice.md |
| "**invoice stats** (counts / amounts by status)" | `easybooks invoice stats [--year <YYYY>]` | references/invoice.md |
| "list my **invoices** [that are unpaid/draft]" | `easybooks invoices list [--status <s>]` | references/invoice.md |
| "is EasyBooks **healthy** / which backend am I on / token still valid" | `easybooks --json doctor` (local config + backend round-trip + version) | this file |
| "**connect** EasyBooks / save my platform token / set it up" | `references/connect.md` skill → user-local hidden entry via `easybooks login --token-stdin [--base-url <url>]` | references/connect.md |
| "EasyBooks **out of date**?" | `easybooks --json doctor --no-fetch --check-upgrade` | references/connect.md |

Routing detail below is supplementary — start with this table.

## §B. Resolving the `easybooks` binary

The plugin ships a Rust CLI binary that is NOT placed on `PATH` automatically by either Codex or Claude Code. Throughout these skills, `<easybooks>` or the literal token `easybooks` mean **"the bundled binary at this resolved path"**, not a `PATH` lookup.

**Resolution order (use the first that resolves to an existing executable):**

1. **`$EASYBOOKS_BIN`** — explicit override. Honor if set.
2. **Claude Code plugin dir** — `$CLAUDE_PLUGIN_ROOT/bin/<platform>/easybooks` (Claude Code sets `CLAUDE_PLUGIN_ROOT` when invoking a plugin's skill).
3. **Codex plugin cache** — `$HOME/.codex/plugins/cache/jacky-plugins/easybooks/<highest-version>/bin/<platform>/easybooks` where `<highest-version>` is the highest version dir present and `<platform>` matches the OS/arch.
4. **`command -v easybooks`** — if the user has installed it on PATH manually (last resort).

The public bundle currently supports `darwin-arm64` and `win32-x64` (binary is `easybooks.exe` on Windows). Other hosts require an explicit trusted `EASYBOOKS_BIN` or PATH installation.

**Portable resolver (bash; works on darwin / linux; for Windows agents use the PowerShell variant below):**

```bash
# Detect platform → cache subdir name used by both codex and claude.
case "$(uname -s)-$(uname -m)" in
  Darwin-arm64)  PLAT=darwin-arm64 ;;
  *) PLAT= ;;
esac

# Walk a search list in priority order; pick first existing executable.
EASYBOOKS_BIN_RESOLVED=""
_cand_paths=(
  "${EASYBOOKS_BIN:-}"
  "${CLAUDE_PLUGIN_ROOT:+${PLAT:+$CLAUDE_PLUGIN_ROOT/bin/$PLAT/easybooks}}"
)
# Codex cache may have several version dirs; agent picks the *highest* one.
# Use python sort (universally available) — POSIX `sort -V` is not portable.
_codex_root="$HOME/.codex/plugins/cache/jacky-plugins/easybooks"
if [ -d "$_codex_root" ]; then
  _latest_codex=$(python3 - "$_codex_root" <<'PY' 2>/dev/null
import os, sys
root = sys.argv[1]
def keyfn(d):
    try:    return tuple(int(x) for x in d.split('.'))
    except: return (-1,)
dirs = [d for d in os.listdir(root) if os.path.isdir(os.path.join(root, d))]
dirs.sort(key=keyfn)
print(os.path.join(root, dirs[-1]) if dirs else "", end="")
PY
)
  [ -n "$_latest_codex" ] && [ -n "$PLAT" ] && _cand_paths+=("$_latest_codex/bin/$PLAT/easybooks")
fi
_cand_paths+=("$(command -v easybooks 2>/dev/null)")

for _p in "${_cand_paths[@]}"; do
  if [ -n "$_p" ] && [ -x "$_p" ]; then EASYBOOKS_BIN_RESOLVED="$_p"; break; fi
done

if [ -z "$EASYBOOKS_BIN_RESOLVED" ]; then
  echo "EasyBooks CLI not found on this host. Install the easybooks plugin first." >&2
  return 1 2>/dev/null || exit 1
fi
export EASYBOOKS_BIN="$EASYBOOKS_BIN_RESOLVED"
"$EASYBOOKS_BIN" --help >/dev/null || { echo "EasyBooks CLI at $EASYBOOKS_BIN is not runnable" >&2; return 1 2>/dev/null || exit 1; }
```

**Windows PowerShell variant** (codex on Windows installs to `$env:USERPROFILE\.codex\...`):

```powershell
$plat = "win32-x64"
$cands = @($env:EASYBOOKS_BIN)
if ($env:CLAUDE_PLUGIN_ROOT) { $cands += "$env:CLAUDE_PLUGIN_ROOT\bin\$plat\easybooks.exe" }
$codexRoot = "$env:USERPROFILE\.codex\plugins\cache\jacky-plugins\easybooks"
if (Test-Path $codexRoot) {
  $latest = Get-ChildItem $codexRoot -Directory | Where-Object { $_.Name -match '^\d+(\.\d+){1,3}$' } | Sort-Object { [version]$_.Name } | Select-Object -Last 1
  if ($latest) { $cands += "$($latest.FullName)\bin\$plat\easybooks.exe" }
}
$cands += (Get-Command easybooks -ErrorAction SilentlyContinue).Source
$env:EASYBOOKS_BIN = $cands | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
```

Once `$EASYBOOKS_BIN` is set, **every command in this doc and every other EasyBooks skill** that starts with the bare token `easybooks` should be invoked as `"$EASYBOOKS_BIN"` (bash) / `& $env:EASYBOOKS_BIN` (PowerShell). The bare `easybooks` token is shorthand throughout these docs; the resolution rule applies uniformly.

**Trust-boundary note**: the `command -v easybooks` fallback trusts the ambient PATH. Prefer the explicit cache paths above when both are available; PATH lookup is the last resort, not the canonical answer.

**Why this matters**: writing `easybooks <subcommand>` assuming PATH is set silently fails in Codex (the binary lives in cache, not PATH). Resolve once at session start; don't waste tool calls hunting for it.

## §C. File-drop / paste decision tree (the core "record this document" path)

Use this whenever the user gives you a file or pastes content: receipt photo, scanned PDF, supplier invoice PDF, Excel/CSV export, bank/credit-card statement, an email, or just plain text describing a purchase.

1. **Agent parses the source locally.** Read the Excel/CSV table, OCR the image, extract PDF text, read the email body — whatever it takes to get the facts. The CLI does **not** read local file paths; you do.
2. **Map each line to an Entry JSON object** (shape in §2). One receipt usually = one Entry; a statement / spreadsheet = many Entries in one batch.
3. **Assign a stable `source_id` per entry** so re-imports are idempotent (invoice number, statement line hash, Gmail message id, etc.). Never omit it for document imports.
4. **Dry-run first:** `easybooks tx import-json --json '<json>' --dry-run`. This validates + echoes the resolved rows without writing.
5. **Show the user the resolved rows** (amount, date, category, classification, type). Get confirmation, especially for amounts and business-vs-personal classification.
6. **Record for real:** rerun the same command **without** `--dry-run`. Output is `{ "created":n, "existing":n, "skipped":n, "skipped_personal":n, "processed":n }` — `existing` > 0 means idempotency already had those rows; that is success, not an error. `skipped_personal` > 0 means the server **dropped** entries that resolved to personal (see §1 taxonomy / self-learning) — they were intentionally not booked.

Single quick one-off (no file)? Use `easybooks income add` / `easybooks expense add` directly instead of building batch JSON. Full detail: `references/record.md`.

## 2. Data shapes (do not invent fields)

**Entry** (used by `tx import-json` and `gmail record`):

```json
{
  "type": "income|expense",
  "amount_cents": 12000,
  "description": "Adobe Creative Cloud",
  "date": "2026-05-01",
  "category_name": "Software",          // optional; backend resolves to id
  "classification": "business|mixed|personal", // optional; see taxonomy below — don't silently default to business
  "source_type": "receipt|invoice|email|statement", // optional
  "source_id": "stable-unique-id",      // REQUIRED for idempotency on imports
  "source_payload": { "from": "billing@vendor.com" }, // include sender `from` so the classifier learns by sender
  "receipt": { "filename": "rcpt.pdf", "content_type": "application/pdf", "content_base64": "<base64>" } // optional; original document (or attach later with `tx attach-receipt`)
}
```

### Classification taxonomy + self-learning (READ)

Three deductibility labels — never just two, and never silently default an unknown to `business`:

| Label | Meaning |
|---|---|
| `business` | Fully deductible — wholly a business cost |
| `mixed` | Partially deductible — split business/personal use |
| `personal` | Pure personal — not deductible |

If unsure, ask one specific question; if it stays unknown, mark it **needs-review** (don't guess `business`).

**Self-learning:** correcting a classification with `easybooks tx reclassify <id> --class <label> --learn` teaches the system to remember that **sender** (keyed off `source_payload.from`), so future transactions from the same sender are classified automatically. Correct once → it remembers. Aggregator senders (e.g. `paypal.com`, `stripe.com`) forward many different merchants, so the system does **not** learn on those — classify those per-receipt.

**Pure-personal is NOT recorded (server-side drop):** an entry that resolves to `personal` is **not booked at all** — no transaction row, no receipt upload — and is returned under `skipped_personal` instead of `created`. It resolves to personal when EITHER the entry carries `classification: "personal"`, OR its sender is one the user has already taught is pure-personal (a confident learned rule, via `reclassify --class personal --learn`). So after you teach a sender as personal, future emails from it are auto-dropped, not recorded. Still submit such entries with `source_payload.from` (don't pre-filter in the agent) — the server decides and reports the count. This applies only to `personal`; `mixed` and not-yet-confident senders are still recorded for review.

`tx import-json` / `gmail record` JSON envelope (the user is identified by the platform token, so no owner id):
```json
{ "source_system": "gmail|receipt-drop|...", "entries": [ <Entry>... ] }
```

**Invoice create** JSON (see `references/invoice.md`):
```json
{
  "client": { "name": "Acme Co", "email": "ap@acme.co" },   // or { "client_id": "<uuid>" }
  "issue_date": "2026-05-01", "due_date": "2026-05-31",
  "tax_rate": 13,
  "items": [ { "description": "Consulting", "quantity": 10, "unit_price": 150 } ],
  "notes": "Net 30", "payment_details": "...", "source_id": "<optional>"
}
```
Server computes subtotal / tax / total and generates the `INV-` invoice number. Amounts in invoice items are decimals (dollars); transaction Entries use integer `amount_cents`.

## §R. Rules — auto-categorization (QB Bank Rules style)

Rules auto-classify / categorize transactions the way QuickBooks Bank Rules do: a transaction is matched against ordered rules and the first matches set its category and/or classification. Use this surface when the user wants to **automate** categorization ("auto-categorize anything from `<sender>` as `<category>`", "make everything matching X business", "set up a rule", "list / disable my rules", "apply my rules to existing transactions"). For a one-off correction of a single transaction, use `tx reclassify` instead — rules are for the repeating case.

| User intent | Command | Endpoint |
|---|---|---|
| "list my rules" | `easybooks rules list` | `GET /api/integrations/rules` |
| "show rule `<id>`" | `easybooks rules show <rule_id>` | `GET /api/integrations/rules/{id}` |
| "create a rule" | `easybooks rules create --json '<rule json>'` | `POST /api/integrations/rules` |
| "delete rule `<id>`" | `easybooks rules delete <rule_id>` | `DELETE /api/integrations/rules/{id}` |
| "enable / turn on rule `<id>`" | `easybooks rules enable <rule_id>` | `PATCH /api/integrations/rules/{id}` `{enabled:true}` |
| "disable / pause rule `<id>`" | `easybooks rules disable <rule_id>` | `PATCH /api/integrations/rules/{id}` `{enabled:false}` |
| "apply / run my rules over transactions" | `easybooks rules apply --scope <all\|unclassified\|selected> [--ids a,b] [--rule-ids r1,r2] [--only-auto-apply] [--commit]` | `POST /api/integrations/rules/apply` |

**Rule shape** (do not invent fields): `name`, `priority` (integer, **lower = evaluated first**), `enabled`, `match_type` (`all` = every condition must hold | `any` = at least one), `apply_to` (`income` | `expense` | `both`), `auto_apply`, `stop_on_match`, `conditions[]`, `actions[]`.

- **condition**: `field` ∈ `description` | `amount` | `type` | `sender_domain`; `operator` ∈ `contains` | `not_contains` | `equals` | `not_equals` | `starts_with` | `ends_with` | `gt` | `gte` | `lt` | `lte`; `value` (string).
- **action**: `action_type` `set_category` (with `category_id`) **OR** `set_classification` (with `classification` ∈ `business` | `mixed` | `personal`). Resolve `category_id` first via `easybooks categories list` — never invent it.

**`rules apply` defaults to a PREVIEW (dry-run)** — it reports what *would* change without writing. Add `--commit` to actually write. `--scope` is `all` | `unclassified` | `selected` (pair `selected` with `--ids a,b`); `--rule-ids` limits which rules run; `--only-auto-apply` restricts to rules whose `auto_apply` is true. Always show the preview to the user before re-running with `--commit`.

**Keys:** `rules list` / `rules show` need a **read** key; `create` / `delete` / `enable` / `disable` / `apply` need a **read_write** key.

Compact `create --json` example — auto-mark anything from a vendor domain as business:
```json
{ "name": "ACME → business", "priority": 100, "enabled": true, "match_type": "all", "apply_to": "expense", "auto_apply": true, "stop_on_match": true,
  "conditions": [ { "field": "sender_domain", "operator": "contains", "value": "acme.com" } ],
  "actions": [ { "action_type": "set_classification", "classification": "business" } ] }
```

## 3. Skill router by user intent

| User says (any phrasing) | Skill | Entry point |
|---|---|---|
| "connect / set up EasyBooks / save my key" | [connect](references/connect.md) | `easybooks login` then `whoami` / `doctor` |
| "log an expense / income", "record this receipt / file / image / PDF / statement", "fix a classification", "attach a receipt" | [record](references/record.md) | `expense add` / `income add` / `tx import-json` / `tx reclassify` / `tx attach-receipt` |
| "create an invoice", "send invoice X", "list my clients / invoices" | [invoice](references/invoice.md) | `invoice create` / `invoice send` / `clients` / `invoices list` |
| "scan my Gmail for receipts / invoices and record them" | [gmail](references/gmail.md) | Gmail MCP read → `gmail record` |
| "is my plugin healthy / which backend am I on" | this file | `easybooks --json doctor` |

## 4. Complete CLI surface by responsibility

| Responsibility | Commands |
|---|---|
| Connect / health | `login`, `whoami`, `doctor` |
| List / search transactions | `tx list [--type income\|expense] [--classification business\|mixed\|personal\|unclassified] [--review needs_review\|reviewed] [--from YYYY-MM-DD] [--to YYYY-MM-DD] [--query <q>] [--limit n]`, `tx receipt-url <id>`, `tx confirm <id>` |
| Record transactions | `income add ...`, `expense add ...`, `tx import-json --json '<json>' [--dry-run]` |
| Edit / classify transactions | `tx update <id> [--amount|--date|--description|--category|--notes] [--dry-run]`, `tx reclassify <id> --class business\|mixed\|personal [--learn]`, `tx attach-receipt <id> --file <path>` |
| Dashboard / summaries | `dashboard [--year YYYY]` |
| Categories | `categories list [--type income\|expense]`, `categories create --name <n> --type income\|expense [--tax-deductible]` |
| Clients | `clients list`, `clients create --name <n> [--email|--phone|--address|--notes]`, `clients update <id> ...`, `clients delete <id>` |
| Invoices | `invoice get <id>`, `invoice create --json '<json>' [--dry-run]`, `invoice send <invoice_id>`, `invoice mark <id> --status paid\|unpaid`, `invoice pdf <id> [--out <path>]`, `invoice stats [--year YYYY]`, `invoices list [--status <s>]` |
| Rules (auto-categorization) | `rules list`, `rules show <id>`, `rules create --json '<json>'`, `rules delete <id>`, `rules enable\|disable <id>`, `rules apply --scope <all\|unclassified\|selected> [--ids ..] [--rule-ids ..] [--only-auto-apply] [--commit]` (preview unless `--commit`) — see §R |
| Gmail (v1) | `gmail record --json '<json>' [--dry-run]` (alias of `tx import-json`, `source_system` defaults to `gmail`), `gmail sync` (v1 stub) |

Treat `<easybooks> --help` as runtime truth when docs and code drift.

## 5. Execution mode boundary — local vs backend

| Step | Where it runs | Network |
|---|---|---|
| Parsing files / OCR / reading Excel-CSV-PDF-email | **Local (agent)** | none |
| Reading Gmail candidate messages | **Local (Gmail MCP)** | the MCP's own |
| `categories/clients/invoices list/find`, `whoami`, all records, `tx reclassify`, `tx attach-receipt`, `invoice create/send`, `gmail record` | **Backend** call (HTTP to the configured base-url) | required |
| `doctor --no-fetch` | **Local** config read | none |
| `doctor` (default) / `whoami` | **Backend** round-trip | required |

The CLI talks to the EasyBooks backend integration endpoints under `/api/integrations/...`. The agent never hits those endpoints itself.

## 6. Default execution behavior

- If intent is unambiguous AND the operation is **not** a money mutation that needs review (a single tiny `expense add` with all fields known), you may run it directly.
- For **document imports and invoices**, always `--dry-run` first and show the resolved rows/totals before the real write. Money accuracy matters more than a round-trip.
- For **`invoice send`** (emails a client) treat it as destructive: confirm once with the user, then run.
- For **ambiguous intent** (which category? business or personal? which client?), ask one specific clarifying question rather than guessing.
- When the CLI returns a structured error with a `hint`, surface it verbatim — the CLI is the source of truth for what to try next.

### 6.1 Parallelize reads, serialize the config write

The CLI is stateless per invocation. Run independent **reads** in parallel — e.g. `categories list` + `clients list` + `invoices list` when staging an invoice. The only thing that is **serial** is `login` (it writes the shared token slot `~/.jackyzhang.app/token/user.json` plus the product runtime config). Recording / invoice writes are independent across distinct `source_id`s, but prefer a single batch `tx import-json` over many parallel `expense add` calls so idempotency and the created/existing counts stay coherent.

## §G. Governance — production gate (REQUIRED, surface to user)

- EasyBooks is governed by the current platform-vault `eb` project card and shared plugin policy. Those canonical files outrank this skill if they change.
- The CLI **defaults to the PROD backend** (`https://easybooks.jackyzhang.app`) — the immicore Go eb-plugin reached via the eb frontend domain's nginx `/api` proxy. The legacy Node backend on `http://localhost:8080` is no longer the default. For non-production work, override to **test** (`https://easybooks-test.jackyzhang.app`) or **LAN** (`http://192.168.1.69:8310`) via `--base-url`.
- Because the default is production, **any write is a production write** and is gated. Before a production mutation, require the explicit current-session authorization named by the project card. If it is absent, stop; do not invent an artifact or reuse an older approval.
- The user's platform token is a secret. Never print or log it. It lives only in the shared slot `~/.jackyzhang.app/token/user.json` (mode 0600). Show only masked identifiers.

## 8. Token & secret rules

- **Never log the token value.** Show only masked identifiers in any output you give the user.
- The credential is the user's durable platform Portal token (`jz_...`), shared by every official Jacky plugin and stored at `~/.jackyzhang.app/token/user.json`. It is sent as `Authorization: Bearer` and both authenticates and identifies the user. Retired `eb_live_` product keys are rejected by the CLI.
- The token lives only in the shared slot `~/.jackyzhang.app/token/user.json` (Windows: `%USERPROFILE%\.jackyzhang.app\token\user.json`). Captured once by any official Jacky plugin.
- Do not write the key anywhere else, do not include it in example commands, do not echo it back.
- Recording, creating invoices, and sending require write scope; read commands need read. A scope/permission error from the CLI means the user's platform token lacks write scope — have them recheck their Portal token, not mint an EasyBooks API key.
