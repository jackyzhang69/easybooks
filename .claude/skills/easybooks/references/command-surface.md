<!-- generated-by: assert-skill-surface.py ; do not edit -->
# easybooks command reference

Generated from `easybooks commands --json` (version `0.5.28`). Every public command is listed.

## Global options

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `--base-url` | option | no |  | Override the configured backend base URL for this invocation |
| `--quiet` | flag | no |  | Reserved for parity with formbro; currently a no-op (output is already machine-first JSON / structured errors) |

## `easybooks login`

Save the Portal token read from stdin to the shared token slot and store the backend URL; returns the saved paths and changes local settings only

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `--token-stdin` | flag | yes |  | Read the personal EasyBooks API key from standard input. The value is never accepted as a command-line argument or printed |
| `--base-url` | option | no |  | Backend base URL. When omitted, resolved at login time through the documented precedence (contract §6): `--base-url` → `$EASYBOOKS_API_URL` → DEFAULT (PROD `https://easybooks.jackyzhang.app`). Override for test/LAN. No clap default so the env tier is honoured instead of silently persisting PROD |

## `easybooks whoami`

Confirm the saved login works; returns the signed-in account and backend reachability, and changes nothing

No arguments.

## `easybooks doctor`

Report this install's version, config, cache freshness and (unless --no-fetch) a live backend check as JSON; --repair-install instead reinstalls this package as the only installed copy

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `--no-fetch` | flag | no |  | Pure local read: only report binary version + config presence + cache freshness. No network access. This is the form agents call at session start |
| `--repair-install` | flag | no |  | Reinstall this package as the only installed copy: switch `~/.jackyzhang.app/plugins/easybooks/current` to it, then delete every other tree and older versions in the host plugin cache. Login, settings and user data are kept. Run it with the binary from the newly installed package. Prints JSON; works offline and without a login |

## `easybooks categories list`

List bookkeeping categories (name and id), optionally only income or expense ones; changes nothing

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `--type` | option | no | `income`, `expense` | Only categories of this type |

## `easybooks categories create`

Create a new category and return it

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `--name` | option | yes |  | Category name |
| `--type` | option | yes | `income`, `expense` | Whether the category is for income or expense |
| `--tax-deductible` | flag | no |  | Mark the category as tax-deductible |

## `easybooks clients list`

List all clients (name and id); changes nothing

No arguments.

## `easybooks clients find`

Find clients whose details match a text query; returns the matches and changes nothing

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `--query` | option | yes |  | Text to match against client names and details |

## `easybooks clients create`

Create a new client from individual flags or a raw JSON object, and return it

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `--name` | option | no |  | Client name |
| `--email` | option | no |  | Client email address |
| `--phone` | option | no |  | Client phone number |
| `--address` | option | no |  | Client postal address |
| `--notes` | option | no |  | Free-text notes about the client |
| `--json` | option | no |  | Post a raw JSON object instead of individual flags |

## `easybooks clients update`

Update the provided fields of one client and return the updated client

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `<CLIENT_ID>` | positional | yes |  | Id of the client, from `clients list` or `clients find` |
| `--name` | option | no |  | New client name |
| `--email` | option | no |  | New email address |
| `--phone` | option | no |  | New phone number |
| `--address` | option | no |  | New postal address |
| `--notes` | option | no |  | New notes |

## `easybooks clients delete`

Permanently delete one client; refuses unless --force is given

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `<CLIENT_ID>` | positional | yes |  | Id of the client, from `clients list` or `clients find` |
| `--force` | flag | no |  | Required guard: confirm the deletion (non-interactive CLI) |

## `easybooks invoices list`

List invoices (id, client, status, totals), optionally only one status; changes nothing

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `--status` | option | no |  | Only invoices in this status, such as paid or unpaid |

## `easybooks income add`

Record one income entry and return the backend's result; --dry-run previews without writing

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `--amount` | option | yes |  | Amount in dollars as a decimal, such as 12.50 |
| `--description` | option | yes |  | What the entry is for |
| `--date` | option | yes |  | Entry date, YYYY-MM-DD |
| `--category` | option | no |  | Category name; the backend resolves it to a category id |
| `--classification` | option | no | `business`, `personal` | Whether the entry is business or personal |
| `--source-system` | option | no |  | Name of the system the entry came from, used for idempotency with --source-id |
| `--source-id` | option | no |  | Id of the entry in the source system; a repeat with the same value is not recorded twice |
| `--notes` | option | no |  | Free-text notes |
| `--dry-run` | flag | no |  | Print the entry that would be sent without recording it |

## `easybooks expense add`

Record one expense entry and return the backend's result; --dry-run previews without writing

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `--amount` | option | yes |  | Amount in dollars as a decimal, such as 12.50 |
| `--description` | option | yes |  | What the entry is for |
| `--date` | option | yes |  | Entry date, YYYY-MM-DD |
| `--category` | option | no |  | Category name; the backend resolves it to a category id |
| `--classification` | option | no | `business`, `personal` | Whether the entry is business or personal |
| `--source-system` | option | no |  | Name of the system the entry came from, used for idempotency with --source-id |
| `--source-id` | option | no |  | Id of the entry in the source system; a repeat with the same value is not recorded twice |
| `--notes` | option | no |  | Free-text notes |
| `--dry-run` | flag | no |  | Print the entry that would be sent without recording it |

## `easybooks tx import-json`

Record a batch of entries from a JSON document ({ source_system, entries:[...] }); --dry-run previews without writing

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `--json` | option | yes |  | JSON document: { "source_system": "...", "entries": [ ... ] } |
| `--dry-run` | flag | no |  | Print the batch that would be sent without recording it |

## `easybooks tx reclassify`

Change one transaction's classification to business, mixed or personal; --learn makes the backend remember it for that sender

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `<TRANSACTION_ID>` | positional | yes |  | The transaction id to reclassify |
| `--class` | option | yes | `business`, `mixed`, `personal` | New classification: business \| mixed \| personal |
| `--learn` | flag | no |  | Teach the system to remember this sender's classification |

## `easybooks tx attach-receipt`

Attach a receipt image or PDF (up to 10 MB) to one transaction; the file is read locally and uploaded

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `<TRANSACTION_ID>` | positional | yes |  | The transaction id to attach the receipt to |
| `--file` | option | yes |  | Path to the receipt file (png/jpg/jpeg/gif/webp/heic/heif/pdf) |

## `easybooks tx list`

List transactions, optionally filtered by type, classification, review state, dates or text; returns matches and changes nothing

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `--type` | option | no | `income`, `expense` | Only income or only expense transactions |
| `--classification` | option | no | `business`, `mixed`, `personal`, `unclassified` | Only transactions with this classification |
| `--review` | option | no |  | Only transactions in this review state: needs_review or reviewed |
| `--from` | option | no |  | Only transactions on or after this date, YYYY-MM-DD |
| `--to` | option | no |  | Only transactions on or before this date, YYYY-MM-DD |
| `--query` | option | no |  | Full-text search query (sent as `q`) |
| `--limit` | option | no |  | Maximum number of transactions to return |

## `easybooks tx receipt-url`

Get a time-limited signed URL for one transaction's attached receipt; changes nothing

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `<TRANSACTION_ID>` | positional | yes |  | The transaction id |
| `--expires` | option | no |  | Signed-URL expiry in seconds |

## `easybooks tx confirm`

Mark one transaction as reviewed (clears needs-review) without changing its classification

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `<TRANSACTION_ID>` | positional | yes |  | The transaction id to confirm |

## `easybooks tx update`

Change the provided fields of one transaction; --dry-run prints the change without sending it

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `<TRANSACTION_ID>` | positional | yes |  | The transaction id to update |
| `--amount` | option | no |  | New amount in dollars as a decimal |
| `--date` | option | no |  | New date, YYYY-MM-DD |
| `--description` | option | no |  | New description |
| `--category` | option | no |  | Category name (maps to `category_name` in the request body) |
| `--notes` | option | no |  | New notes |
| `--dry-run` | flag | no |  | Print the PATCH body without calling the backend |

## `easybooks tx delete`

Permanently delete one transaction; refuses unless --force is given

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `<TRANSACTION_ID>` | positional | yes |  | The transaction id to delete |
| `--force` | flag | no |  | Required guard: confirm the deletion (non-interactive CLI) |

## `easybooks invoice create`

Create an invoice from a JSON document and return it with the server-computed totals; --dry-run previews without saving

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `--json` | option | yes |  | JSON document: client or client_id, issue_date, due_date, items, optional tax_rate and notes |
| `--dry-run` | flag | no |  | Validate and show the computed totals without creating the invoice |

## `easybooks invoice send`

Email one invoice to its client and return the backend's answer; this contacts the client

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `<INVOICE_ID>` | positional | yes |  | The invoice id |

## `easybooks invoice get`

Fetch one invoice with its items and totals; changes nothing

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `<INVOICE_ID>` | positional | yes |  | The invoice id |

## `easybooks invoice mark`

Mark one invoice as paid or unpaid and return the result

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `<INVOICE_ID>` | positional | yes |  | The invoice id |
| `--status` | option | yes |  | New status: paid or unpaid |

## `easybooks invoice pdf`

Download one invoice as a PDF file and print the saved path

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `<INVOICE_ID>` | positional | yes |  | The invoice id |
| `--out` | option | no |  | Output file path (default: ./<filename from response>) |

## `easybooks invoice stats`

Show invoice totals by status, optionally for one year; changes nothing

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `--year` | option | no |  | Only invoices of this year, YYYY |

## `easybooks gmail record`

Record receipts and invoices that an agent extracted from Gmail (same as `tx import-json` with source_system gmail); --dry-run previews without writing

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `--json` | option | yes |  | JSON document: { "entries": [ ... ] }, each entry's source_id being the Gmail message id |
| `--dry-run` | flag | no |  | Print the batch that would be sent without recording it |

## `easybooks gmail sync`

Placeholder that only reports that native Gmail sync is not available yet; changes nothing

No arguments.

## `easybooks rules list`

List auto-categorization rules; changes nothing

No arguments.

## `easybooks rules show`

Show one rule's definition; changes nothing

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `<RULE_ID>` | positional | yes |  | The rule id |

## `easybooks rules create`

Create a rule from a JSON rule definition and return it

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `--json` | option | yes |  | JSON rule definition |

## `easybooks rules delete`

Permanently delete one rule

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `<RULE_ID>` | positional | yes |  | The rule id |

## `easybooks rules enable`

Turn one rule on

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `<RULE_ID>` | positional | yes |  | The rule id |

## `easybooks rules disable`

Turn one rule off

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `<RULE_ID>` | positional | yes |  | The rule id |

## `easybooks rules apply`

Preview which transactions the rules would reclassify; only --commit writes the changes

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `--scope` | option | yes |  | Selection scope: all \| unclassified \| selected |
| `--ids` | option | no |  | Comma-separated transaction ids (used with `--scope selected`) |
| `--rule-ids` | option | no |  | Comma-separated rule ids to limit which rules run |
| `--only-auto-apply` | flag | no |  | Only run rules flagged for auto-apply |
| `--commit` | flag | no |  | Persist matches instead of returning a dry-run preview |

## `easybooks dashboard`

Show the dashboard statistics summary, optionally for one year; returns the stats and changes nothing

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `--year` | option | no |  | Filter stats to a specific year (YYYY) |

## `easybooks feedback create`

Send a feature request, bug report or knowledge tip to Jacky and return the created report; needs the user's confirmation flag

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `--title` | option | yes |  | Short title (<=200 chars) |
| `--description` | option | yes |  | Description body |
| `--kind` | option | no |  | Kind of report: feature-request, bug-report or knowledge-tip |
| `--idempotency-key` | option | yes |  | Required idempotency key for safe retries |
| `--user-confirmed` | flag | no |  | Explicit user confirmation gate (skills must pass this) |

## `easybooks feedback status`

Fetch one feedback report you sent by its id; returns its current status and changes nothing

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `--report-id` | option | yes |  | Id of the report, from `feedback create` |

## `easybooks pair join`

Join a support connection with the code from Jacky's assistant, only after the user agrees; returns the connection state

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `--code` | option | yes |  | Code that Jacky's assistant gave the user |
| `--user-confirmed` | flag | no |  | Pass this only after the user explicitly agreed to connect |
| `--json` | flag | no |  | Print JSON instead of plain text |

## `easybooks pair status`

Show whether a support connection is open; changes nothing

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `--json` | flag | no |  | Print JSON instead of plain text |

## `easybooks pair snapshot`

Send the current product envelope (a JSON snapshot of the user's EasyBooks state) to Jacky's assistant

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `--envelope-json` | option | yes |  | JSON envelope file; `-` reads stdin |
| `--json` | flag | no |  | Print JSON instead of plain text |

## `easybooks pair inbox`

Show unread instructions from Jacky's assistant; does not mark them read

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `--json` | flag | no |  | Print JSON instead of plain text |

## `easybooks pair read`

Mark one instruction as read after you have shown it to the user

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `--message-id` | option | yes |  | Id of the instruction, from `pair inbox` |
| `--json` | flag | no |  | Print JSON instead of plain text |

## `easybooks pair result`

Send the outcome of an action Jacky's assistant asked for, as success or failure

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `--ok` | flag | no |  | Pass when the requested action succeeded; omit it to report a failure |
| `--error` | option | no |  | Short error text to send when the action failed |
| `--envelope-json` | option | no |  | Optional JSON envelope file with the result details; `-` reads stdin |
| `--json` | flag | no |  | Print JSON instead of plain text |

## `easybooks pair close`

End the support connection

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `--reason` | option | no |  | Short reason for closing, such as done |
| `--json` | flag | no |  | Print JSON instead of plain text |

## `easybooks commands`

Print the complete public command surface of this binary as JSON (schema plugin-cli-commands-v2); changes nothing

| Argument | Kind | Required | Values | Meaning |
|---|---|---|---|---|
| `--json` | flag | no |  | Always JSON; accepted so `easybooks commands --json` is the contract |
