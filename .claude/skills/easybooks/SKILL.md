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
  platform token, tell Jacky, connect with Jacky, join code from Jacky,
  pair session. One discovery file; playbooks in references/.
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
  "what categories do I have", "tell Jacky",
  "connect with Jacky", "join code from Jacky", "pair session".
---

# EasyBooks plugin — agent consumption contract

## Talk to the human (mandatory)

Speak in everyday words. Say what you are doing and what happens next. Do not
lead with binary paths, `--help`, or raw JSON. Keep CLI and JSON between
tools. Never show credentials, bank data, or receipt contents in chat unless
the user asked for that detail. On the first session after install, read
[references/get-started.md](references/get-started.md) before your first
user-visible reply. After a version update, read
[references/whats-new.md](references/whats-new.md) and resume the prior intent
without asking the human to repeat it. Never ask the human to run doctor as
homework. Run the intended ordinary command first; when stdout is
`jz.plugin.envelope.v1`, follow its exact `status` and `continue_args` until
the product reports `ready`, then resume the sealed request. Never assign
doctor homework when an envelope already names the route.

## Live CLI surface (fail-closed)

The capability list below is the complete command surface of this version.
`"$EASYBOOKS_BIN" commands --json` prints the same surface and confirms it;
volatile data (categories, clients, invoices, transactions, rules) always comes
from the live list commands, never from memory.

```bash
"$EASYBOOKS_BIN" commands --json
easybooks --json doctor
```

Output of every command is JSON already. A top-level `--json` written before the
command name, as in `easybooks --json doctor`, is a harmless no-op and not an
option of `doctor`; the option lists below show each command's own flags only.

## Capabilities (complete surface of this version)

Setup and health:

- `login --token-stdin [--base-url <url>]` — reads one token line from stdin and saves the shared Portal token; never pass the token as an argument.
- `whoami` — shows which account is signed in and whether the backend answers.
- `doctor [--no-fetch]` — health report as JSON: version, config, plugin cache freshness and (without `--no-fetch`) a live backend check.
- `doctor --repair-install` — makes the package that this binary belongs to the only installed copy; works offline and without a login (see Updates).
- `commands --json` — prints this surface as JSON.

Look things up:

- `categories list [--type income|expense]` — lists categories with ids.
- `categories create --name <n> --type income|expense [--tax-deductible]` — creates a category.
- `clients list` — lists clients.
- `clients find --query <q>` — finds clients by text.
- `clients create [--name <n>] [--email <e>] [--phone <p>] [--address <a>] [--notes <t>] [--json '<object>']` — creates a client from flags or a raw JSON object.
- `clients update <client_id> [--name <n>] [--email <e>] [--phone <p>] [--address <a>] [--notes <t>]` — changes the given client fields.
- `clients delete <client_id> --force` — permanently deletes a client; refuses without `--force`.
- `invoices list [--status <s>]` — lists invoices, optionally one status.
- `dashboard [--year <YYYY>]` — shows the dashboard statistics summary.

Record income and expenses:

- `income add --amount <dollars> --description <t> --date <YYYY-MM-DD> [--category <name>] [--classification business|personal] [--source-system <s>] [--source-id <id>] [--notes <t>] [--dry-run]` — records one income entry.
- `expense add` — same flags as `income add`; records one expense entry.
- `tx import-json --json '<json>' [--dry-run]` — records a batch of entries from `{ "source_system", "entries": [...] }`.
- `tx list [--type income|expense] [--classification business|mixed|personal|unclassified] [--review needs_review|reviewed] [--from <YYYY-MM-DD>] [--to <YYYY-MM-DD>] [--query <q>] [--limit <n>]` — finds transactions.
- `tx reclassify <transaction_id> --class business|mixed|personal [--learn]` — corrects a classification; `--learn` remembers it for that sender.
- `tx attach-receipt <transaction_id> --file <path>` — uploads a receipt image or PDF (up to 10 MB).
- `tx receipt-url <transaction_id> [--expires <seconds>]` — returns a signed link to the receipt.
- `tx confirm <transaction_id>` — marks a transaction reviewed without changing it.
- `tx update <transaction_id> [--amount <d>] [--date <YYYY-MM-DD>] [--description <t>] [--category <name>] [--notes <t>] [--dry-run]` — changes the given fields.
- `tx delete <transaction_id> --force` — permanently deletes a transaction; refuses without `--force`.
- `gmail record --json '<json>' [--dry-run]` — records receipts that an agent extracted from Gmail (`source_id` is the Gmail message id).
- `gmail sync` — placeholder that reports native Gmail sync is not available yet.

Invoices:

- `invoice create --json '<json>' [--dry-run]` — creates an invoice; `--dry-run` shows the computed totals without saving.
- `invoice send <invoice_id>` — emails the invoice to the client (confirm first).
- `invoice get <invoice_id>` — shows one invoice.
- `invoice mark <invoice_id> --status paid|unpaid` — marks an invoice paid or unpaid.
- `invoice pdf <invoice_id> [--out <path>]` — saves the invoice PDF and prints the path.
- `invoice stats [--year <YYYY>]` — shows invoice totals by status.

Auto-categorization rules:

- `rules list` — lists rules.
- `rules show <rule_id>` — shows one rule.
- `rules create --json '<rule json>'` — creates a rule.
- `rules delete <rule_id>` — deletes a rule.
- `rules enable <rule_id>` — turns a rule on.
- `rules disable <rule_id>` — turns a rule off.
- `rules apply --scope all|unclassified|selected [--ids <a,b>] [--rule-ids <r1,r2>] [--only-auto-apply] [--commit]` — previews what the rules would change; only `--commit` writes.

Tell Jacky and live support:

- `feedback create --title <t> --description <t> --idempotency-key <k> --user-confirmed [--kind feature-request|bug-report|knowledge-tip]` — sends a report to Jacky after the user confirmed the draft.
- `feedback status --report-id <id>` — shows a report that was sent.
- `pair join --code <CODE> --user-confirmed [--json]` — opens a support connection after the user agreed.
- `pair status [--json]` — shows whether a connection is open.
- `pair snapshot --envelope-json <path|-> [--json]` — sends the current product envelope; `-` reads it from stdin.
- `pair inbox [--json]` — shows unread instructions from Jacky's assistant.
- `pair read --message-id <id> [--json]` — marks one instruction read.
- `pair result [--ok] [--error <text>] [--envelope-json <path|->] [--json]` — reports the outcome of a requested action.
- `pair close [--reason <text>] [--json]` — ends the connection.

## Agent router — intents

| User intent | Host does | Human may be asked |
|---|---|---|
| "what can EasyBooks do" | Read the capability list above; `"$EASYBOOKS_BIN" commands --json` confirms it; translate record/invoice/Gmail capabilities | Connect once if not logged in ([connect](references/connect.md)) |
| record this receipt / log expense or income / import spreadsheet or statement | [record](references/record.md): parse locally, dry-run import, confirm, write | Amount, date, business vs personal, category when ambiguous |
| create / send invoice / list clients or invoices | [invoice](references/invoice.md) | Client and line items; confirm before `invoice send` emails a client |
| scan Gmail for receipts | [gmail](references/gmail.md) | Confirm parsed rows before write |
| list categories / clients / invoices / dashboard | Live list/read commands per [command-router](references/command-router.md) | Clarify filters when ambiguous |
| connect EasyBooks / save platform token | [connect](references/connect.md): agent pipes token via stdin | Token file path or one-time paste (never argv) |
| tell Jacky | [tell-jacky](references/tell-jacky.md) | Confirm exact draft before send |
| connect with Jacky / pair session / join code from Jacky | [pair-session](references/pair-session.md) | Confirm once that Jacky's assistant may look at this machine's EasyBooks status |

Playbooks: [connect](references/connect.md), [record](references/record.md),
[invoice](references/invoice.md), [gmail](references/gmail.md),
[tell-jacky](references/tell-jacky.md),
[pair-session](references/pair-session.md).

Command router and CLI paths (not for first-session orientation):
[references/command-router.md](references/command-router.md).

EasyBooks is a self-employed (Canadian) finance app: income/expense transactions,
categories, clients, and invoices. This plugin lets an agent record bookkeeping
through one bundled CLI instead of touching the database or backend directly.

## Binary

Prefer `$EASYBOOKS_BIN` if it points at a runnable `easybooks` binary. Otherwise
use the canonical current package
`~/.jackyzhang.app/plugins/easybooks/current/bin/<platform>/easybooks`
(`easybooks.exe` on `win32-x64`); `<platform>` is `darwin-arm64`, `linux-x64` or
`win32-x64`. `command -v easybooks` / PATH is last resort
only. Export `EASYBOOKS_BIN` once per session; every later command uses that path.

## Operating rules

1. **All EasyBooks operations go through the bundled CLI.** Do not call the
   backend or write a database directly. Local file parsing is allowed only
   before the CLI boundary.
2. **Never guess ids.** Record with names, or list first
   (`categories list`, `clients find`, `invoices list`).
3. **Money path is dry-run → confirm → write.** Document imports and invoices
   always `--dry-run` first. Confirm amounts and business vs personal. Then
   rerun without `--dry-run`. `invoice send` emails a client — confirm once.
4. **Idempotency is mandatory for recorded documents.** Every parsed receipt /
   invoice / email row carries a stable `source_id`. Re-running the same import
   must not double-record. For Gmail, `source_id` is the Gmail message id.
5. **One shared platform token.** Login is `login --token-stdin` only. The
   durable credential is `~/.jackyzhang.app/token/user.json`. Do not create a
   second durable token. Never print the raw value. Connect details:
   [connect](references/connect.md).
6. **Production is the default.** Writes are gated. Follow the current
   platform-vault project card before a production mutation.

Entry JSON, invoice JSON, classification, and auto-categorization rules live in
the playbooks above — do not invent fields. When the CLI returns a structured
error with a `hint`, surface it verbatim.

## Updates

`easybooks` may print one stderr line with schema `jz.plugin.update_notice.v1`. It never blocks a command. Finish what the user asked first, then tell them once, in plain words, that a newer version is available and ask whether to update now. On yes: refresh this plugin through the host (Claude Code: `claude plugin marketplace update jacky-plugins`, then `claude plugin update easybooks@jacky-plugins`; Codex: `codex plugin marketplace upgrade jacky-plugins`, then `codex plugin add easybooks@jacky-plugins`; other hosts: attach the new `plugins/easybooks/` tree). Then run `doctor --repair-install` with the binary from the **new** package, not the folder this session started from (Claude Code: `~/.claude/plugins/cache/jacky-plugins/easybooks/<latest>/bin/<platform>/easybooks`; Codex: `~/.codex/plugins/cache/jacky-plugins/easybooks/<latest>/bin/<platform>/easybooks`; `<latest>` is the notice's `latest` value or the highest version folder present; `<platform>` is `darwin-arm64`, `linux-x64` or `win32-x64`). Read `references/whats-new.md` from that folder and tell the user in one or two sentences what changed and that the host loads it fully after a restart. For the rest of this session use that folder for every command and reference; the old version folder is gone. On no: do not ask again in this session.
