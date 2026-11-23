## Shared platform token (host agent — mandatory)

- Canonical durable user credential: `~/.jackyzhang.app/token/user.json` (`jz_` only).
- **One connect for the whole platform.** If this file already exists from any other official plugin, do **not** ask the human to log in again.
- On load, if `user.json` is missing but a legacy on-disk `jz_` exists (`token/user.json`, product-local config token fields), **migrate immediately** by running this product's CLI `whoami`/`doctor`/any authenticated command (load-time migrate is built in). User must feel no re-paste.
- Only when no migratable `jz_` exists: complete connect once using the token-delivery rules below. After that, every registered plugin reuses the same slot.
- Never print or log the raw token. Prefer masked doctor/whoami output.
- Do not create product-local durable token files.

## Token delivery to the host agent (connect) — LOCKED 2026-08-24

Same contract as every official plugin. The host agent performs connect **for** the human. **Never** tell the human to open a terminal and run login commands themselves. This block is the whole connect path; later steps must not replace it with a human-typed terminal prompt.

Accept input in this order:

1. **File containing the token (preferred).** If the human provides a filesystem path (e.g. `~/Desktop/jacky-token.txt`) or an attached/readable file whose contents are a single `jz_…` value (optional surrounding whitespace/newline only):
   - Read the file in the agent tool channel.
   - Pipe the token to the product CLI via stdin only:

```bash
printf %s "$(cat -- "$TOKEN_FILE")" | "$EASYBOOKS_BIN" login --token-stdin [--base-url <BASE_URL>]
```

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


# Connect EasyBooks

## Skill load order (resolves any apparent contradiction)

Two different time-scales:
- **`references/connect.md` (THIS skill)** is the **one-time setup ritual** — runs once per user/machine to capture the user's platform Portal token (`jz_`) + base-url and verify cache freshness. Then it tells you to load `the easybooks router`.
- **`the easybooks router`** is the **every-session reference contract** — load it (and keep it loaded) for every interaction. Its description says "READ THIS FIRST" because, once setup is done, capabilities is what an agent reads first on each subsequent session.

So: **`references/connect.md` once → then `the easybooks router` every session**, including the very first one. Both can be true.

## Already-connected fast path

If `~/.jackyzhang.app/token/user.json` exists — from **any** official Jacky plugin, not just EasyBooks — AND `easybooks whoami` returns ok, the user is already connected. Skip the token capture (step 1) and the login step (step 4) — **but still run the doctor self-check (step 3)** because plugin upgrades happen out-of-band and you should detect a stale cache on every session start.

## What this does (first-time path)

Persists the user's platform Portal token (`jz_`) + backend base-url through the bundled `easybooks` CLI so that every subsequent skill (record / invoice / gmail) can call the EasyBooks backend without ever seeing the raw token again. The token both authenticates and identifies the user — there is no owner id to capture. It is the **same token every official Jacky plugin uses**, so if the user connected FormBro, AnyChat, AnyPDF, or AnyDoc, EasyBooks needs no new credential.

Two pieces are captured:
- **Platform token** — the user's durable Portal token, starts with `jz_`. It is the **same token every official Jacky plugin uses** (FormBro, AnyChat, AnyPDF, AnyDoc, EasyBooks), stored once at `~/.jackyzhang.app/token/user.json`. If that file already exists, the user is connected — do not ask for a token again and do not tell them EasyBooks needs its own. Retired `eb_live_` product keys are **rejected by the CLI** (`EasyBooks accepts only platform jz_ credentials`); never send the user to the web app to mint one.
- **Base-url** — which backend to talk to. **Default `https://easybooks.jackyzhang.app` (PROD)** — the immicore Go eb-plugin, reached via the eb frontend domain's nginx `/api` proxy (`/api/integrations/*`). Test backend is `https://easybooks-test.jackyzhang.app` (immicore-test); a LAN dev backend is e.g. `http://192.168.1.69:8310`. Because the default is production, every write is a production write — see the governance note below.

## How it works

1. **Obtain the user's platform Portal token (`jz_`)** — but first check whether `~/.jackyzhang.app/token/user.json` already exists; if it does, skip this step entirely and never ask again. Do **not** send the user to the EasyBooks web app to mint an API key: retired `eb_live_` product keys are rejected by the CLI. Follow the token-delivery rules at the top of this skill (file path preferred, chat paste allowed with one warning). Confirm only the backend target. **If they don't specify, the default is `https://easybooks.jackyzhang.app` (PROD).** For test work pass `https://easybooks-test.jackyzhang.app`; for a LAN dev backend pass e.g. `http://192.168.1.69:8310`.
2. **Resolve the bundled `easybooks` binary** — defer to `the easybooks router/SKILL.md` (prefer `$EASYBOOKS_BIN`, then `~/.jackyzhang.app/plugins/easybooks/current/bin/<platform>/easybooks`; PATH last). Set `$EASYBOOKS_BIN` in the shell once; every subsequent command in every EasyBooks skill uses that exact path, not an ambient `PATH` lookup.
3. **Plugin cache freshness self-check (mandatory):**

   ```sh
   "$EASYBOOKS_BIN" --json doctor --no-fetch
   ```

   - `--no-fetch` is **required** — it skips any bootstrap/network IO and guarantees a pure local cache read.
   - Newer marketplace versions are announced by the CLI itself (see the Updates section of the router `SKILL.md`); this check does not look for them.

   **Failure handling — before parsing JSON:**
   - If the command exits non-zero **OR** prints anything that doesn't parse as JSON: abort the connect flow. Surface the raw output with the message "EasyBooks CLI doctor check failed — the bundled binary may not be installed correctly. Verify your plugin install and re-run." **Do not proceed to step 4 (login).**

   **Branching on the parsed JSON** (in order — first matching branch wins):

   - `cache.location == "not_in_cache"`: dev install or manual install. Proceed silently.
   - `cache.stale == false`: cache is current. Proceed silently.
   - `cache.stale == true`: tell the user verbatim:

     > Your EasyBooks plugin cache is stale: running version `<cache.version>` from cache at `<cache.location>`, latest available `<cache.latest_available>`. Refresh via your plugin manager (codex: re-sync `jacky-plugins`; claude: reinstall the plugin) before proceeding, or this session may hit known-fixed bugs.
     >
     > If you want to proceed anyway: say so explicitly and I'll continue.

     Wait for user direction before continuing.
   - Any other shape (missing fields, unexpected `cache.location` value): proceed but warn — log "doctor returned unexpected cache shape; continuing".

   This check is cheap (single local filesystem scan; no network IO because of `--no-fetch`) and saves the user from chasing already-fixed bugs across the rest of the session. Do not skip it.

4. The host agent runs login. Follow **Token delivery** above. File preferred:

   ```sh
   printf %s "$(cat -- "$TOKEN_FILE")" | "$EASYBOOKS_BIN" login --token-stdin [--base-url <BASE_URL>]
   ```

   - Do not tell the user to run this in their own terminal. Do not use `--token`. The agent pipes stdin.
   - If the user did not give a base-url, the CLI default is `https://easybooks.jackyzhang.app` (PROD). For test, pass `--base-url https://easybooks-test.jackyzhang.app`; for LAN dev, pass e.g. `--base-url http://192.168.1.69:8310`. Since the default is production, confirm the intended target with the user before login.
   - Output is JSON with a masked identifier only. The CLI writes the token to the shared slot `~/.jackyzhang.app/token/user.json` (mode 0600) and the base-url to the product runtime config. Never echo the raw token.

5. Verify by running:

   ```sh
   "$EASYBOOKS_BIN" whoami
   ```

   On success the backend echoes the user id and scope, with the credential masked. On failure the token is invalid/revoked or the base-url is wrong/unreachable — have the user recheck the token and base-url. Retired `eb_live_` keys are rejected outright with a message naming `jz_`.

## Token rules — never break

- **Never log the token value.** Show only masked identifiers in any output you give the user, including the example `login` command.
- **Never write the key into any file other than the CLI's own `config.json`.** That file is what `login --token-stdin` writes; do not write your own copy elsewhere.
- **Never embed the token into prompts, tool descriptions, or example commands** you generate. Always use a masked placeholder when you show example commands.
- The token is the user's personal platform credential, shared across every official Jacky plugin. If exposed, the user must revoke and reissue it in Portal — revoking it affects every plugin, so say so plainly.

## Governance — production is the default, writes are gated (surface this)

- The CLI **now defaults to the PROD backend** (`https://easybooks.jackyzhang.app`) — the immicore Go eb-plugin behind the eb frontend nginx `/api` proxy. The legacy Node backend on `http://localhost:8080` is no longer the default.
- Because the default is production, **any write you perform is a production write.** Follow the current platform-vault project card and the user's explicit current-session authorization for production mutations.
- For ordinary non-production work, point at **test** (`https://easybooks-test.jackyzhang.app`, served by immicore-test) or a LAN dev backend (e.g. `http://192.168.1.69:8310`) via `--base-url`.
- If the current-session authorization required by the project card is absent, stop before a production mutation. Confirm the base-url before login so you don't silently persist production for routine work.

## After this skill succeeds

Tell the user:

> Connected to EasyBooks at `<base_url>` as user `<user_id>` (scope `<scope>`). Platform token stored in the shared slot (masked). Plugin v`<doctor.binary_version>` ready.
>
> **Quick reference** (you can ask me for any of these):
> - "record this receipt / log a $120 software expense on 2026-05-01" — parse → record
> - "create an invoice for `<client>` for `<items>`" — staged with a dry-run first
> - "scan my Gmail for receipts and record them" — read via Gmail, record idempotently
> - "is EasyBooks healthy / which backend am I on" — runs `easybooks --json doctor`

Then (silently, no narration) load `the easybooks router/SKILL.md`. It tells you:
- Which skill to call for any user intent (the top router)
- Binary resolution and the local-parse → CLI-record boundary
- Entry / invoice JSON shapes in the record and invoice playbooks
- The production governance gate
