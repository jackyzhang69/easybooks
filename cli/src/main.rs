mod client;
mod commands;
mod config;
mod doctor;
mod identity;
mod lifecycle;
mod output;
mod pair;
mod signals;

// The binary/version resolver lives in the lib crate so it can be unit-tested
// in isolation; re-export the path the binary uses.
use easybooks_cli::bootstrap;

use anyhow::Result;
use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};
use commands::{
    clients, dashboard, gmail, invoices, read, rules, setup, transactions, tx_ops, tx_query,
};

#[derive(Parser)]
#[command(name = "easybooks")]
#[command(about = "EasyBooks CLI — the only boundary for EasyBooks reads/writes")]
#[command(version)]
struct Cli {
    /// Override the configured backend base URL for this invocation.
    #[arg(long, global = true)]
    base_url: Option<String>,
    /// Accepted for parity; output is JSON by default. Declared with a distinct
    /// argument id (`json_output`) so it never collides with the per-command
    /// `--json <payload>` args (tx import-json / invoice create / gmail record),
    /// and intentionally NOT `global` so it does not propagate into — and shadow
    /// — those subcommand args. Pass it at the top level (e.g. `easybooks --json
    /// doctor`); it is a no-op since output is already machine-first JSON.
    #[arg(long = "json", id = "json_output", default_value_t = true)]
    json: bool,
    /// Reserved for parity with formbro; currently a no-op (output is already
    /// machine-first JSON / structured errors).
    #[arg(long, global = true, default_value_t = false)]
    quiet: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Save the Portal token read from stdin to the shared token slot and store the backend URL; returns the saved paths and changes local settings only.
    Login(LoginArgs),
    /// Confirm the saved login works; returns the signed-in account and backend reachability, and changes nothing.
    Whoami,
    /// Report this install's version, config, cache freshness and (unless --no-fetch) a live backend check as JSON; --repair-install instead reinstalls this package as the only installed copy.
    Doctor(doctor::DoctorArgs),
    /// Look up bookkeeping categories, or create one.
    Categories(CategoriesCommand),
    /// Look up, create, update or delete clients.
    Clients(ClientsCommand),
    /// List invoices, optionally filtered by status.
    Invoices(InvoicesCommand),
    /// Record a single income entry.
    Income(IncomeCommand),
    /// Record a single expense entry.
    Expense(ExpenseCommand),
    /// Import, list, correct, confirm or delete transactions.
    Tx(TxCommand),
    /// Create, read, send, mark or download invoices.
    Invoice(InvoiceCommand),
    /// Record receipts and invoices that an agent extracted from Gmail.
    Gmail(GmailCommand),
    /// List, create, delete, toggle or apply auto-categorization rules.
    Rules(RulesCommand),
    /// Show the dashboard statistics summary, optionally for one year; returns the stats and changes nothing.
    Dashboard(DashboardCommand),
    /// Send product feedback to Jacky or look up a report you sent.
    Feedback(FeedbackCommand),
    /// Open a live support connection with Jacky's assistant and exchange messages over it.
    #[command(subcommand)]
    Pair(PairCmd),
    /// Private operator surface (Tell Jacky admin feedback). Requires admin.json.
    #[command(hide = true)]
    Admin(AdminCommand),
    /// Print the complete public command surface of this binary as JSON (schema plugin-cli-commands-v2); changes nothing.
    #[command(name = "commands")]
    CliCommands {
        /// Always JSON; accepted so `easybooks commands --json` is the contract.
        #[arg(long, default_value_t = false)]
        json: bool,
    },
}

#[derive(Args)]
struct AdminCommand {
    #[command(subcommand)]
    command: AdminSubcommand,
}

#[derive(Subcommand)]
enum AdminSubcommand {
    /// Install platform admin jz_ into ~/.jackyzhang.app/token/admin.json (stdin).
    Login(AdminLoginArgs),
    /// Tell Jacky admin feedback for plugin_id=easybooks
    Feedback(AdminFeedbackCommand),
}

#[derive(Args)]
struct AdminLoginArgs {
    #[arg(long, help = "Read exactly one admin jz_ line from stdin")]
    token_stdin: bool,
}

#[derive(Args)]
struct AdminFeedbackCommand {
    #[command(subcommand)]
    command: AdminFeedbackSubcommand,
}

#[derive(Subcommand)]
enum AdminFeedbackSubcommand {
    List {
        #[arg(long)]
        status: Option<String>,
        #[arg(long, conflicts_with = "status")]
        all: bool,
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    Get {
        #[arg(long)]
        id: String,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    Triage {
        #[arg(long)]
        id: String,
        #[arg(long)]
        message: String,
        #[arg(long)]
        request_key: String,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    Close {
        #[arg(long)]
        id: String,
        #[arg(long)]
        message: String,
        #[arg(long)]
        request_key: String,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    DirectClose {
        #[arg(long)]
        id: String,
        #[arg(long)]
        ack_message: String,
        #[arg(long)]
        resolution_message: String,
        #[arg(long)]
        request_key: String,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
}

#[derive(Args)]
struct FeedbackCommand {
    #[command(subcommand)]
    command: FeedbackSubcommand,
}

#[derive(Subcommand)]
enum FeedbackSubcommand {
    /// Send a feature request, bug report or knowledge tip to Jacky and return the created report; needs the user's confirmation flag.
    Create(FeedbackCreateArgs),
    /// Fetch one feedback report you sent by its id; returns its current status and changes nothing.
    Status(FeedbackStatusArgs),
}

#[derive(Subcommand)]
enum PairCmd {
    /// Join a support connection with the code from Jacky's assistant, only after the user agrees; returns the connection state.
    Join {
        /// Code that Jacky's assistant gave the user.
        #[arg(long)]
        code: String,
        /// Pass this only after the user explicitly agreed to connect.
        #[arg(long, default_value_t = false)]
        user_confirmed: bool,
        /// Print JSON instead of plain text.
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Show whether a support connection is open; changes nothing.
    Status {
        /// Print JSON instead of plain text.
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Send the current product envelope (a JSON snapshot of the user's EasyBooks state) to Jacky's assistant.
    Snapshot {
        /// JSON envelope file; `-` reads stdin
        #[arg(long, value_name = "PATH")]
        envelope_json: String,
        /// Print JSON instead of plain text.
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Show unread instructions from Jacky's assistant; does not mark them read.
    Inbox {
        /// Print JSON instead of plain text.
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Mark one instruction as read after you have shown it to the user.
    Read {
        /// Id of the instruction, from `pair inbox`.
        #[arg(long)]
        message_id: String,
        /// Print JSON instead of plain text.
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Send the outcome of an action Jacky's assistant asked for, as success or failure.
    Result {
        /// Pass when the requested action succeeded; omit it to report a failure.
        #[arg(long)]
        ok: bool,
        /// Short error text to send when the action failed.
        #[arg(long)]
        error: Option<String>,
        /// Optional JSON envelope file with the result details; `-` reads stdin
        #[arg(long, value_name = "PATH")]
        envelope_json: Option<String>,
        /// Print JSON instead of plain text.
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// End the support connection.
    Close {
        /// Short reason for closing, such as done.
        #[arg(long, default_value = "done")]
        reason: String,
        /// Print JSON instead of plain text.
        #[arg(long, default_value_t = false)]
        json: bool,
    },
}

#[derive(Args)]
struct FeedbackCreateArgs {
    /// Short title (<=200 chars).
    #[arg(long)]
    title: String,
    /// Description body.
    #[arg(long)]
    description: String,
    /// Kind of report: feature-request, bug-report or knowledge-tip.
    #[arg(long, default_value = "bug-report")]
    kind: String,
    /// Required idempotency key for safe retries.
    #[arg(long)]
    idempotency_key: String,
    /// Explicit user confirmation gate (skills must pass this).
    #[arg(long)]
    user_confirmed: bool,
}

#[derive(Args)]
struct FeedbackStatusArgs {
    /// Id of the report, from `feedback create`.
    #[arg(long)]
    report_id: String,
}

#[derive(Args)]
struct LoginArgs {
    /// Read the personal EasyBooks API key from standard input. The value is
    /// never accepted as a command-line argument or printed.
    #[arg(long, required = true)]
    token_stdin: bool,
    /// Backend base URL. When omitted, resolved at login time through the
    /// documented precedence (contract §6): `--base-url` → `$EASYBOOKS_API_URL`
    /// → DEFAULT (PROD `https://easybooks.jackyzhang.app`). Override for
    /// test/LAN. No clap default so the env tier is honoured instead of silently
    /// persisting PROD.
    #[arg(long)]
    base_url: Option<String>,
}

#[derive(Args)]
struct CategoriesCommand {
    #[command(subcommand)]
    command: CategoriesSub,
}

#[derive(Subcommand)]
enum CategoriesSub {
    /// List bookkeeping categories (name and id), optionally only income or expense ones; changes nothing.
    List {
        /// Only categories of this type.
        #[arg(long = "type", value_enum)]
        type_filter: Option<TxType>,
    },
    /// Create a new category and return it.
    Create {
        /// Category name.
        #[arg(long)]
        name: String,
        /// Whether the category is for income or expense.
        #[arg(long = "type", value_enum)]
        type_: TxType,
        /// Mark the category as tax-deductible.
        #[arg(long = "tax-deductible", default_value_t = false)]
        tax_deductible: bool,
    },
}

#[derive(Args)]
struct ClientsCommand {
    #[command(subcommand)]
    command: ClientsSub,
}

#[derive(Subcommand)]
enum ClientsSub {
    /// List all clients (name and id); changes nothing.
    List,
    /// Find clients whose details match a text query; returns the matches and changes nothing.
    Find {
        /// Text to match against client names and details.
        #[arg(long)]
        query: String,
    },
    /// Create a new client from individual flags or a raw JSON object, and return it.
    Create {
        /// Client name.
        #[arg(long)]
        name: Option<String>,
        /// Client email address.
        #[arg(long)]
        email: Option<String>,
        /// Client phone number.
        #[arg(long)]
        phone: Option<String>,
        /// Client postal address.
        #[arg(long)]
        address: Option<String>,
        /// Free-text notes about the client.
        #[arg(long)]
        notes: Option<String>,
        /// Post a raw JSON object instead of individual flags.
        #[arg(long)]
        json: Option<String>,
    },
    /// Update the provided fields of one client and return the updated client.
    Update {
        /// Id of the client, from `clients list` or `clients find`.
        client_id: String,
        /// New client name.
        #[arg(long)]
        name: Option<String>,
        /// New email address.
        #[arg(long)]
        email: Option<String>,
        /// New phone number.
        #[arg(long)]
        phone: Option<String>,
        /// New postal address.
        #[arg(long)]
        address: Option<String>,
        /// New notes.
        #[arg(long)]
        notes: Option<String>,
    },
    /// Permanently delete one client; refuses unless --force is given.
    Delete {
        /// Id of the client, from `clients list` or `clients find`.
        client_id: String,
        /// Required guard: confirm the deletion (non-interactive CLI).
        #[arg(long, default_value_t = false)]
        force: bool,
    },
}

#[derive(Args)]
struct InvoicesCommand {
    #[command(subcommand)]
    command: InvoicesSub,
}

#[derive(Subcommand)]
enum InvoicesSub {
    /// List invoices (id, client, status, totals), optionally only one status; changes nothing.
    List {
        /// Only invoices in this status, such as paid or unpaid.
        #[arg(long)]
        status: Option<String>,
    },
}

#[derive(Clone, ValueEnum)]
enum TxType {
    Income,
    Expense,
}

impl TxType {
    fn as_str(&self) -> &'static str {
        match self {
            TxType::Income => "income",
            TxType::Expense => "expense",
        }
    }
}

#[derive(Clone, ValueEnum)]
enum Classification {
    Business,
    Personal,
}

impl Classification {
    fn as_str(&self) -> &'static str {
        match self {
            Classification::Business => "business",
            Classification::Personal => "personal",
        }
    }
}

/// Classification filter for `tx list`. Wider than the add-time `Classification`
/// (income/expense add only set business|personal): listing can also filter by
/// `mixed` and `unclassified` (the latter maps to `classification IS NULL`
/// server-side), which is the core "show me what still needs classifying" query.
#[derive(Clone, ValueEnum)]
enum TxListClass {
    Business,
    Mixed,
    Personal,
    Unclassified,
}

impl TxListClass {
    fn as_str(&self) -> &'static str {
        match self {
            TxListClass::Business => "business",
            TxListClass::Mixed => "mixed",
            TxListClass::Personal => "personal",
            TxListClass::Unclassified => "unclassified",
        }
    }
}

/// Reclassification label for an already-recorded transaction. Unlike the
/// `Classification` used on `income/expense add` (business/personal only), a
/// correction may set `mixed` — a partially deductible transaction.
#[derive(Clone, ValueEnum)]
enum ReclassClass {
    Business,
    Mixed,
    Personal,
}

impl ReclassClass {
    fn as_str(&self) -> &'static str {
        match self {
            ReclassClass::Business => "business",
            ReclassClass::Mixed => "mixed",
            ReclassClass::Personal => "personal",
        }
    }
}

/// Shared flag set for `income add` / `expense add`.
#[derive(Args)]
struct EntryAddArgs {
    /// Amount in dollars as a decimal, such as 12.50.
    #[arg(long)]
    amount: String,
    /// What the entry is for.
    #[arg(long)]
    description: String,
    /// Entry date, YYYY-MM-DD.
    #[arg(long)]
    date: String,
    /// Category name; the backend resolves it to a category id.
    #[arg(long)]
    category: Option<String>,
    /// Whether the entry is business or personal.
    #[arg(long, value_enum)]
    classification: Option<Classification>,
    /// Name of the system the entry came from, used for idempotency with --source-id.
    #[arg(long = "source-system")]
    source_system: Option<String>,
    /// Id of the entry in the source system; a repeat with the same value is not recorded twice.
    #[arg(long = "source-id")]
    source_id: Option<String>,
    /// Free-text notes.
    #[arg(long)]
    notes: Option<String>,
    /// Print the entry that would be sent without recording it.
    #[arg(long = "dry-run", default_value_t = false)]
    dry_run: bool,
}

impl EntryAddArgs {
    fn into_add_args(self) -> transactions::AddArgs {
        transactions::AddArgs {
            amount: self.amount,
            description: self.description,
            date: self.date,
            category: self.category,
            classification: self.classification.map(|c| c.as_str().to_string()),
            source_system: self.source_system,
            source_id: self.source_id,
            notes: self.notes,
            dry_run: self.dry_run,
        }
    }
}

#[derive(Args)]
struct IncomeCommand {
    #[command(subcommand)]
    command: IncomeSub,
}

#[derive(Subcommand)]
enum IncomeSub {
    /// Record one income entry and return the backend's result; --dry-run previews without writing.
    Add(EntryAddArgs),
}

#[derive(Args)]
struct ExpenseCommand {
    #[command(subcommand)]
    command: ExpenseSub,
}

#[derive(Subcommand)]
enum ExpenseSub {
    /// Record one expense entry and return the backend's result; --dry-run previews without writing.
    Add(EntryAddArgs),
}

#[derive(Args)]
struct TxCommand {
    #[command(subcommand)]
    command: TxSub,
}

#[derive(Subcommand)]
enum TxSub {
    /// Record a batch of entries from a JSON document ({ source_system, entries:[...] }); --dry-run previews without writing.
    ImportJson {
        /// JSON document: { "source_system": "...", "entries": [ ... ] }.
        #[arg(long)]
        json: String,
        /// Print the batch that would be sent without recording it.
        #[arg(long = "dry-run", default_value_t = false)]
        dry_run: bool,
    },
    /// Change one transaction's classification to business, mixed or personal; --learn makes the backend remember it for that sender.
    Reclassify {
        /// The transaction id to reclassify.
        transaction_id: String,
        /// New classification: business | mixed | personal.
        #[arg(long = "class", value_enum)]
        class: ReclassClass,
        /// Teach the system to remember this sender's classification.
        #[arg(long, default_value_t = false)]
        learn: bool,
    },
    /// Attach a receipt image or PDF (up to 10 MB) to one transaction; the file is read locally and uploaded.
    AttachReceipt {
        /// The transaction id to attach the receipt to.
        transaction_id: String,
        /// Path to the receipt file (png/jpg/jpeg/gif/webp/heic/heif/pdf).
        #[arg(long)]
        file: String,
    },
    /// List transactions, optionally filtered by type, classification, review state, dates or text; returns matches and changes nothing.
    List {
        /// Only income or only expense transactions.
        #[arg(long = "type", value_enum)]
        type_filter: Option<TxType>,
        /// Only transactions with this classification.
        #[arg(long, value_enum)]
        classification: Option<TxListClass>,
        /// Only transactions in this review state: needs_review or reviewed.
        #[arg(long)]
        review: Option<String>,
        /// Only transactions on or after this date, YYYY-MM-DD.
        #[arg(long)]
        from: Option<String>,
        /// Only transactions on or before this date, YYYY-MM-DD.
        #[arg(long)]
        to: Option<String>,
        /// Full-text search query (sent as `q`).
        #[arg(long)]
        query: Option<String>,
        /// Maximum number of transactions to return.
        #[arg(long)]
        limit: Option<u32>,
    },
    /// Get a time-limited signed URL for one transaction's attached receipt; changes nothing.
    ReceiptUrl {
        /// The transaction id.
        transaction_id: String,
        /// Signed-URL expiry in seconds.
        #[arg(long)]
        expires: Option<u32>,
    },
    /// Mark one transaction as reviewed (clears needs-review) without changing its classification.
    Confirm {
        /// The transaction id to confirm.
        transaction_id: String,
    },
    /// Change the provided fields of one transaction; --dry-run prints the change without sending it.
    Update {
        /// The transaction id to update.
        transaction_id: String,
        /// New amount in dollars as a decimal.
        #[arg(long)]
        amount: Option<String>,
        /// New date, YYYY-MM-DD.
        #[arg(long)]
        date: Option<String>,
        /// New description.
        #[arg(long)]
        description: Option<String>,
        /// Category name (maps to `category_name` in the request body).
        #[arg(long)]
        category: Option<String>,
        /// New notes.
        #[arg(long)]
        notes: Option<String>,
        /// Print the PATCH body without calling the backend.
        #[arg(long = "dry-run", default_value_t = false)]
        dry_run: bool,
    },
    /// Permanently delete one transaction; refuses unless --force is given.
    Delete {
        /// The transaction id to delete.
        transaction_id: String,
        /// Required guard: confirm the deletion (non-interactive CLI).
        #[arg(long, default_value_t = false)]
        force: bool,
    },
}

#[derive(Args)]
struct InvoiceCommand {
    #[command(subcommand)]
    command: InvoiceSub,
}

#[derive(Subcommand)]
enum InvoiceSub {
    /// Create an invoice from a JSON document and return it with the server-computed totals; --dry-run previews without saving.
    Create {
        /// JSON document: client or client_id, issue_date, due_date, items, optional tax_rate and notes.
        #[arg(long)]
        json: String,
        /// Validate and show the computed totals without creating the invoice.
        #[arg(long = "dry-run", default_value_t = false)]
        dry_run: bool,
    },
    /// Email one invoice to its client and return the backend's answer; this contacts the client.
    Send {
        /// The invoice id.
        invoice_id: String,
    },
    /// Fetch one invoice with its items and totals; changes nothing.
    Get {
        /// The invoice id.
        invoice_id: String,
    },
    /// Mark one invoice as paid or unpaid and return the result.
    Mark {
        /// The invoice id.
        invoice_id: String,
        /// New status: paid or unpaid.
        #[arg(long)]
        status: String,
    },
    /// Download one invoice as a PDF file and print the saved path.
    Pdf {
        /// The invoice id.
        invoice_id: String,
        /// Output file path (default: ./<filename from response>).
        #[arg(long)]
        out: Option<String>,
    },
    /// Show invoice totals by status, optionally for one year; changes nothing.
    Stats {
        /// Only invoices of this year, YYYY.
        #[arg(long)]
        year: Option<u32>,
    },
}

#[derive(Args)]
struct GmailCommand {
    #[command(subcommand)]
    command: GmailSub,
}

#[derive(Subcommand)]
enum GmailSub {
    /// Record receipts and invoices that an agent extracted from Gmail (same as `tx import-json` with source_system gmail); --dry-run previews without writing.
    Record {
        /// JSON document: { "entries": [ ... ] }, each entry's source_id being the Gmail message id.
        #[arg(long)]
        json: String,
        /// Print the batch that would be sent without recording it.
        #[arg(long = "dry-run", default_value_t = false)]
        dry_run: bool,
    },
    /// Placeholder that only reports that native Gmail sync is not available yet; changes nothing.
    Sync,
}

#[derive(Args)]
struct RulesCommand {
    #[command(subcommand)]
    command: RulesSub,
}

#[derive(Args)]
struct DashboardCommand {
    /// Filter stats to a specific year (YYYY).
    #[arg(long)]
    year: Option<u32>,
}

#[derive(Subcommand)]
enum RulesSub {
    /// List auto-categorization rules; changes nothing.
    List,
    /// Show one rule's definition; changes nothing.
    Show {
        /// The rule id.
        rule_id: String,
    },
    /// Create a rule from a JSON rule definition and return it.
    Create {
        /// JSON rule definition.
        #[arg(long)]
        json: String,
    },
    /// Permanently delete one rule.
    Delete {
        /// The rule id.
        rule_id: String,
    },
    /// Turn one rule on.
    Enable {
        /// The rule id.
        rule_id: String,
    },
    /// Turn one rule off.
    Disable {
        /// The rule id.
        rule_id: String,
    },
    /// Preview which transactions the rules would reclassify; only --commit writes the changes.
    Apply {
        /// Selection scope: all | unclassified | selected.
        #[arg(long)]
        scope: String,
        /// Comma-separated transaction ids (used with `--scope selected`).
        #[arg(long)]
        ids: Option<String>,
        /// Comma-separated rule ids to limit which rules run.
        #[arg(long = "rule-ids")]
        rule_ids: Option<String>,
        /// Only run rules flagged for auto-apply.
        #[arg(long = "only-auto-apply", default_value_t = false)]
        only_auto_apply: bool,
        /// Persist matches instead of returning a dry-run preview.
        #[arg(long, default_value_t = false)]
        commit: bool,
    },
}

fn main() {
    let result = run();
    if let Err(error) = &result {
        output::print_error(error);
    }
    // The daily update notice runs after the command finished, on success and
    // on failure, and never changes the exit code.
    lifecycle::after_command();
    if result.is_err() {
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    if std::env::args_os().skip(1).any(|argument| {
        let argument = argument.to_string_lossy();
        argument == "--token" || argument.starts_with("--token=")
    }) {
        anyhow::bail!("unsupported option '--token'; use `easybooks login --token-stdin`");
    }
    let cli = Cli::parse();
    let base_url_arg = cli.base_url;
    // `--json` / `--quiet` exist for parity with formbro's global flags. Output
    // is already machine-first JSON and errors are structured, so they are
    // accepted but do not alter behaviour in v1. Bind them so the parsed fields
    // are observed (keeps `-D warnings` clean) and the intent is documented.
    let _parity_flags = (cli.json, cli.quiet);

    // `login` and `doctor` are the two commands that do NOT require an already
    // authenticated client: `login` writes the config; `doctor` tolerates a
    // missing config and runs its own scoped (optional) network probe. Handle
    // them first so they work before/without a successful `Config::load`.
    match cli.command {
        Command::Login(args) => setup::login_from_stdin(args.token_stdin, args.base_url),
        Command::Doctor(args) if args.repair_install => lifecycle::repair_install(),
        Command::Doctor(args) => doctor::run(args, base_url_arg),
        Command::CliCommands { json: _ } => print_plugin_cli_commands(),
        other => dispatch(other, base_url_arg),
    }
}

fn print_plugin_cli_commands() -> Result<()> {
    println!("{}", serde_json::to_string(&plugin_cli_commands_payload())?);
    Ok(())
}

fn plugin_cli_commands_payload() -> serde_json::Value {
    jz_plugin_lifecycle::commands::clap_payload(
        Cli::command(),
        "easybooks",
        env!("CARGO_PKG_VERSION"),
    )
}

/// Build the authenticated client from config and dispatch every command that
/// talks to the backend. Split out so `run` can short-circuit `login`/`doctor`.
fn dispatch(command: Command, base_url_arg: Option<String>) -> Result<()> {
    let original_args: Vec<String> = std::env::args().skip(1).collect();
    // Load first so leftover ~/.easybooks can migrate into the shared slot
    // before the readiness envelope decides the user is disconnected.
    let loaded = config::Config::load(base_url_arg);
    if jz_plugin_common::auth::read_durable_token()
        .ok()
        .flatten()
        .is_none()
    {
        let operation = easybooks_cli::envelope::operation_from_args(&original_args);
        let envelope = easybooks_cli::envelope::connection_not_ready(&operation, original_args);
        easybooks_cli::envelope::print_stdout(&envelope)?;
        return Ok(());
    }
    let config = loaded?;
    let client = client::ApiClient::from_config(&config)?;

    match command {
        Command::Login(_) | Command::Doctor(_) | Command::CliCommands { .. } => unreachable!(),
        Command::Whoami => setup::whoami(&client, &config),
        Command::Categories(cmd) => match cmd.command {
            CategoriesSub::List { type_filter } => {
                read::categories_list(&client, type_filter.map(|t| t.as_str().to_string()))
            }
            CategoriesSub::Create {
                name,
                type_,
                tax_deductible,
            } => read::categories_create(&client, &name, type_.as_str(), tax_deductible),
        },
        Command::Clients(cmd) => match cmd.command {
            ClientsSub::List => read::clients_list(&client),
            ClientsSub::Find { query } => read::clients_find(&client, query),
            ClientsSub::Create {
                name,
                email,
                phone,
                address,
                notes,
                json,
            } => clients::create(
                &client,
                name.as_deref(),
                email.as_deref(),
                phone.as_deref(),
                address.as_deref(),
                notes.as_deref(),
                json.as_deref(),
            ),
            ClientsSub::Update {
                client_id,
                name,
                email,
                phone,
                address,
                notes,
            } => clients::update(
                &client,
                &client_id,
                name.as_deref(),
                email.as_deref(),
                phone.as_deref(),
                address.as_deref(),
                notes.as_deref(),
            ),
            ClientsSub::Delete { client_id, force } => clients::delete(&client, &client_id, force),
        },
        Command::Invoices(cmd) => match cmd.command {
            InvoicesSub::List { status } => read::invoices_list(&client, status),
        },
        Command::Income(cmd) => match cmd.command {
            IncomeSub::Add(args) => transactions::add(&client, "income", args.into_add_args()),
        },
        Command::Expense(cmd) => match cmd.command {
            ExpenseSub::Add(args) => transactions::add(&client, "expense", args.into_add_args()),
        },
        Command::Tx(cmd) => match cmd.command {
            TxSub::ImportJson { json, dry_run } => {
                transactions::import_json(&client, &json, dry_run)
            }
            TxSub::Reclassify {
                transaction_id,
                class,
                learn,
            } => tx_ops::reclassify(&client, &transaction_id, class.as_str(), learn),
            TxSub::AttachReceipt {
                transaction_id,
                file,
            } => tx_ops::attach_receipt(&client, &transaction_id, &file),
            TxSub::List {
                type_filter,
                classification,
                review,
                from,
                to,
                query,
                limit,
            } => tx_query::list(
                &client,
                type_filter.map(|t| t.as_str()),
                classification.map(|c| c.as_str()),
                review.as_deref(),
                from.as_deref(),
                to.as_deref(),
                query.as_deref(),
                limit,
            ),
            TxSub::ReceiptUrl {
                transaction_id,
                expires,
            } => tx_query::receipt_url(&client, &transaction_id, expires),
            TxSub::Confirm { transaction_id } => tx_query::confirm(&client, &transaction_id),
            TxSub::Update {
                transaction_id,
                amount,
                date,
                description,
                category,
                notes,
                dry_run,
            } => tx_query::update(
                &client,
                &transaction_id,
                amount.as_deref(),
                date.as_deref(),
                description.as_deref(),
                category.as_deref(),
                notes.as_deref(),
                dry_run,
            ),
            TxSub::Delete {
                transaction_id,
                force,
            } => tx_query::delete(&client, &transaction_id, force),
        },
        Command::Invoice(cmd) => match cmd.command {
            InvoiceSub::Create { json, dry_run } => invoices::create(&client, &json, dry_run),
            InvoiceSub::Send { invoice_id } => invoices::send(&client, &invoice_id),
            InvoiceSub::Get { invoice_id } => invoices::get(&client, &invoice_id),
            InvoiceSub::Mark { invoice_id, status } => {
                invoices::mark(&client, &invoice_id, &status)
            }
            InvoiceSub::Pdf { invoice_id, out } => {
                invoices::pdf(&client, &invoice_id, out.as_deref())
            }
            InvoiceSub::Stats { year } => invoices::stats(&client, year),
        },
        Command::Gmail(cmd) => match cmd.command {
            GmailSub::Record { json, dry_run } => gmail::record(&client, &json, dry_run),
            GmailSub::Sync => gmail::sync(),
        },
        Command::Rules(cmd) => match cmd.command {
            RulesSub::List => rules::list(&client),
            RulesSub::Show { rule_id } => rules::show(&client, &rule_id),
            RulesSub::Create { json } => rules::create(&client, &json),
            RulesSub::Delete { rule_id } => rules::delete(&client, &rule_id),
            RulesSub::Enable { rule_id } => rules::enable(&client, &rule_id),
            RulesSub::Disable { rule_id } => rules::disable(&client, &rule_id),
            RulesSub::Apply {
                scope,
                ids,
                rule_ids,
                only_auto_apply,
                commit,
            } => rules::apply(
                &client,
                &scope,
                ids.as_deref(),
                rule_ids.as_deref(),
                only_auto_apply,
                commit,
            ),
        },
        Command::Dashboard(cmd) => dashboard::stats(&client, cmd.year),
        Command::Feedback(cmd) => match cmd.command {
            FeedbackSubcommand::Create(args) => commands::feedback::create(
                &client,
                &args.title,
                &args.description,
                &args.kind,
                &args.idempotency_key,
                args.user_confirmed,
            ),
            FeedbackSubcommand::Status(args) => {
                commands::feedback::status(&client, &args.report_id)
            }
        },
        Command::Pair(cmd) => match cmd {
            PairCmd::Join {
                code,
                user_confirmed,
                json,
            } => pair::join(&code, user_confirmed, json),
            PairCmd::Status { json } => pair::status(json),
            PairCmd::Snapshot {
                envelope_json,
                json,
            } => pair::snapshot(&envelope_json, json),
            PairCmd::Inbox { json } => pair::inbox(json),
            PairCmd::Read { message_id, json } => pair::read_message(&message_id, json),
            PairCmd::Result {
                ok,
                error,
                envelope_json,
                json,
            } => pair::result(ok, error.as_deref(), envelope_json.as_deref(), json),
            PairCmd::Close { reason, json } => pair::close(&reason, json),
        },

        Command::Admin(cmd) => match cmd.command {
            AdminSubcommand::Login(args) => {
                if !args.token_stdin {
                    anyhow::bail!("admin login requires --token-stdin");
                }
                commands::admin_feedback::login_admin_from_stdin()
            }
            AdminSubcommand::Feedback(fb) => match fb.command {
                AdminFeedbackSubcommand::List {
                    status,
                    all,
                    limit,
                    json,
                } => commands::admin_feedback::list(status.as_deref(), all, limit, json),
                AdminFeedbackSubcommand::Get { id, json } => {
                    commands::admin_feedback::get(&id, json)
                }
                AdminFeedbackSubcommand::Triage {
                    id,
                    message,
                    request_key,
                    json,
                } => commands::admin_feedback::triage(&id, &message, &request_key, json),
                AdminFeedbackSubcommand::Close {
                    id,
                    message,
                    request_key,
                    json,
                } => commands::admin_feedback::close(&id, &message, &request_key, json),
                AdminFeedbackSubcommand::DirectClose {
                    id,
                    ack_message,
                    resolution_message,
                    request_key,
                    json,
                } => commands::admin_feedback::direct_close(
                    &id,
                    &ack_message,
                    &resolution_message,
                    &request_key,
                    json,
                ),
            },
        },
    }
}

// Keep the resolver reachable from the binary (used by skills' §B blocks at
// runtime and surfaced via doctor's cache check). Referencing it here also
// documents the §0 contract entry point for maintainers.
#[allow(dead_code)]
fn _resolver_entrypoints() {
    let _ = bootstrap::resolve_binary;
    let _ = bootstrap::current_platform;
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::Parser;

    #[test]
    fn admin_feedback_all_is_explicit_and_conflicts_with_status() {
        let default = Cli::try_parse_from(["easybooks", "admin", "feedback", "list"])
            .expect("default feedback list should parse");
        match default.command {
            super::Command::Admin(cmd) => match cmd.command {
                super::AdminSubcommand::Feedback(feedback) => match feedback.command {
                    super::AdminFeedbackSubcommand::List { all, .. } => assert!(!all),
                    _ => panic!("expected feedback list"),
                },
                _ => panic!("expected admin feedback"),
            },
            _ => panic!("expected admin command"),
        }

        let all = Cli::try_parse_from(["easybooks", "admin", "feedback", "list", "--all"])
            .expect("explicit all should parse");
        match all.command {
            super::Command::Admin(cmd) => match cmd.command {
                super::AdminSubcommand::Feedback(feedback) => match feedback.command {
                    super::AdminFeedbackSubcommand::List { all, .. } => assert!(all),
                    _ => panic!("expected feedback list"),
                },
                _ => panic!("expected admin feedback"),
            },
            _ => panic!("expected admin command"),
        }

        assert!(Cli::try_parse_from([
            "easybooks",
            "admin",
            "feedback",
            "list",
            "--all",
            "--status",
            "received",
        ])
        .is_err());
    }

    #[test]
    fn parses_income_add() {
        let cli = Cli::try_parse_from([
            "easybooks",
            "income",
            "add",
            "--amount",
            "120.00",
            "--description",
            "Consulting",
            "--date",
            "2026-05-01",
        ])
        .expect("income add should parse");
        match cli.command {
            super::Command::Income(_) => {}
            _ => panic!("expected income command"),
        }
    }

    #[test]
    fn parses_tx_import_json_dry_run() {
        let cli = Cli::try_parse_from([
            "easybooks",
            "tx",
            "import-json",
            "--json",
            r#"{"source_system":"manual","entries":[]}"#,
            "--dry-run",
        ])
        .expect("tx import-json should parse");
        match cli.command {
            super::Command::Tx(_) => {}
            _ => panic!("expected tx command"),
        }
    }

    #[test]
    fn parses_tx_reclassify() {
        let cli = Cli::try_parse_from([
            "easybooks",
            "tx",
            "reclassify",
            "txn_123",
            "--class",
            "mixed",
            "--learn",
        ])
        .expect("tx reclassify should parse");
        match cli.command {
            super::Command::Tx(_) => {}
            _ => panic!("expected tx command"),
        }
    }

    #[test]
    fn rejects_bad_reclassify_class() {
        let result = Cli::try_parse_from([
            "easybooks",
            "tx",
            "reclassify",
            "txn_123",
            "--class",
            "deductible",
        ]);
        assert!(
            result.is_err(),
            "invalid --class should be rejected by clap"
        );
    }

    #[test]
    fn parses_tx_attach_receipt() {
        let cli = Cli::try_parse_from([
            "easybooks",
            "tx",
            "attach-receipt",
            "txn_123",
            "--file",
            "/tmp/receipt.pdf",
        ])
        .expect("tx attach-receipt should parse");
        match cli.command {
            super::Command::Tx(_) => {}
            _ => panic!("expected tx command"),
        }
    }

    #[test]
    fn parses_gmail_sync() {
        let cli =
            Cli::try_parse_from(["easybooks", "gmail", "sync"]).expect("gmail sync should parse");
        match cli.command {
            super::Command::Gmail(_) => {}
            _ => panic!("expected gmail command"),
        }
    }

    #[test]
    fn parses_rules_show() {
        let cli = Cli::try_parse_from(["easybooks", "rules", "show", "rule_123"])
            .expect("rules show should parse");
        match cli.command {
            super::Command::Rules(_) => {}
            _ => panic!("expected rules command"),
        }
    }

    #[test]
    fn parses_rules_apply() {
        let cli = Cli::try_parse_from([
            "easybooks",
            "rules",
            "apply",
            "--scope",
            "selected",
            "--ids",
            "txn_1,txn_2",
            "--rule-ids",
            "rule_a,rule_b",
            "--only-auto-apply",
            "--commit",
        ])
        .expect("rules apply should parse");
        match cli.command {
            super::Command::Rules(_) => {}
            _ => panic!("expected rules command"),
        }
    }

    #[test]
    fn parses_tx_list_with_filters() {
        let cli = Cli::try_parse_from([
            "easybooks",
            "tx",
            "list",
            "--type",
            "expense",
            "--classification",
            "business",
            "--from",
            "2026-01-01",
            "--to",
            "2026-06-30",
            "--query",
            "coffee",
            "--limit",
            "50",
        ])
        .expect("tx list with filters should parse");
        match cli.command {
            super::Command::Tx(cmd) => match cmd.command {
                super::TxSub::List { limit, query, .. } => {
                    assert_eq!(limit, Some(50));
                    assert_eq!(query.as_deref(), Some("coffee"));
                }
                _ => panic!("expected tx list"),
            },
            _ => panic!("expected tx command"),
        }
    }

    #[test]
    fn parses_tx_list_no_filters() {
        let cli = Cli::try_parse_from(["easybooks", "tx", "list"])
            .expect("tx list without filters should parse");
        match cli.command {
            super::Command::Tx(cmd) => match cmd.command {
                super::TxSub::List { limit, query, .. } => {
                    assert!(limit.is_none());
                    assert!(query.is_none());
                }
                _ => panic!("expected tx list"),
            },
            _ => panic!("expected tx command"),
        }
    }

    #[test]
    fn parses_tx_update_dry_run() {
        let cli = Cli::try_parse_from([
            "easybooks",
            "tx",
            "update",
            "txn_abc",
            "--amount",
            "99.99",
            "--category",
            "Office Supplies",
            "--dry-run",
        ])
        .expect("tx update --dry-run should parse");
        match cli.command {
            super::Command::Tx(cmd) => match cmd.command {
                super::TxSub::Update {
                    transaction_id,
                    dry_run,
                    category,
                    ..
                } => {
                    assert_eq!(transaction_id, "txn_abc");
                    assert!(dry_run);
                    assert_eq!(category.as_deref(), Some("Office Supplies"));
                }
                _ => panic!("expected tx update"),
            },
            _ => panic!("expected tx command"),
        }
    }

    #[test]
    fn parses_tx_delete_force() {
        let cli = Cli::try_parse_from(["easybooks", "tx", "delete", "txn_123", "--force"])
            .expect("tx delete --force should parse");
        match cli.command {
            super::Command::Tx(cmd) => match cmd.command {
                super::TxSub::Delete {
                    transaction_id,
                    force,
                } => {
                    assert_eq!(transaction_id, "txn_123");
                    assert!(force);
                }
                _ => panic!("expected tx delete"),
            },
            _ => panic!("expected tx command"),
        }
    }

    #[test]
    fn parses_invoice_mark() {
        let cli = Cli::try_parse_from([
            "easybooks",
            "invoice",
            "mark",
            "inv_123",
            "--status",
            "paid",
        ])
        .expect("invoice mark should parse");
        match cli.command {
            super::Command::Invoice(cmd) => match cmd.command {
                super::InvoiceSub::Mark { invoice_id, status } => {
                    assert_eq!(invoice_id, "inv_123");
                    assert_eq!(status, "paid");
                }
                _ => panic!("expected invoice mark"),
            },
            _ => panic!("expected invoice command"),
        }
    }

    #[test]
    fn parses_invoice_pdf_with_out() {
        let cli = Cli::try_parse_from([
            "easybooks",
            "invoice",
            "pdf",
            "inv_456",
            "--out",
            "/tmp/invoice.pdf",
        ])
        .expect("invoice pdf --out should parse");
        match cli.command {
            super::Command::Invoice(cmd) => match cmd.command {
                super::InvoiceSub::Pdf { invoice_id, out } => {
                    assert_eq!(invoice_id, "inv_456");
                    assert_eq!(out.as_deref(), Some("/tmp/invoice.pdf"));
                }
                _ => panic!("expected invoice pdf"),
            },
            _ => panic!("expected invoice command"),
        }
    }

    #[test]
    fn parses_clients_create_with_flags() {
        let cli = Cli::try_parse_from([
            "easybooks",
            "clients",
            "create",
            "--name",
            "Acme Corp",
            "--email",
            "billing@acme.com",
        ])
        .expect("clients create should parse");
        match cli.command {
            super::Command::Clients(cmd) => match cmd.command {
                super::ClientsSub::Create { name, email, .. } => {
                    assert_eq!(name.as_deref(), Some("Acme Corp"));
                    assert_eq!(email.as_deref(), Some("billing@acme.com"));
                }
                _ => panic!("expected clients create"),
            },
            _ => panic!("expected clients command"),
        }
    }

    #[test]
    fn parses_clients_create_with_json() {
        let cli = Cli::try_parse_from([
            "easybooks",
            "clients",
            "create",
            "--json",
            r#"{"name":"Test"}"#,
        ])
        .expect("clients create --json should parse");
        match cli.command {
            super::Command::Clients(cmd) => match cmd.command {
                super::ClientsSub::Create { json, .. } => {
                    assert!(json.is_some());
                }
                _ => panic!("expected clients create"),
            },
            _ => panic!("expected clients command"),
        }
    }

    #[test]
    fn parses_clients_delete_force() {
        let cli = Cli::try_parse_from(["easybooks", "clients", "delete", "client_789", "--force"])
            .expect("clients delete --force should parse");
        match cli.command {
            super::Command::Clients(cmd) => match cmd.command {
                super::ClientsSub::Delete { client_id, force } => {
                    assert_eq!(client_id, "client_789");
                    assert!(force);
                }
                _ => panic!("expected clients delete"),
            },
            _ => panic!("expected clients command"),
        }
    }

    #[test]
    fn parses_categories_create() {
        let cli = Cli::try_parse_from([
            "easybooks",
            "categories",
            "create",
            "--name",
            "Office Supplies",
            "--type",
            "expense",
            "--tax-deductible",
        ])
        .expect("categories create should parse");
        match cli.command {
            super::Command::Categories(cmd) => match cmd.command {
                super::CategoriesSub::Create {
                    name,
                    tax_deductible,
                    ..
                } => {
                    assert_eq!(name, "Office Supplies");
                    assert!(tax_deductible);
                }
                _ => panic!("expected categories create"),
            },
            _ => panic!("expected categories command"),
        }
    }

    #[test]
    fn parses_dashboard_with_year() {
        let cli = Cli::try_parse_from(["easybooks", "dashboard", "--year", "2026"])
            .expect("dashboard --year should parse");
        match cli.command {
            super::Command::Dashboard(cmd) => {
                assert_eq!(cmd.year, Some(2026));
            }
            _ => panic!("expected dashboard command"),
        }
    }

    #[test]
    fn parses_dashboard_no_year() {
        let cli = Cli::try_parse_from(["easybooks", "dashboard"])
            .expect("dashboard without year should parse");
        match cli.command {
            super::Command::Dashboard(cmd) => {
                assert!(cmd.year.is_none());
            }
            _ => panic!("expected dashboard command"),
        }
    }

    #[test]
    fn commands_json_schema_lists_public_paths_and_omits_admin() {
        let payload = super::plugin_cli_commands_payload();
        assert_eq!(payload["schema"], "plugin-cli-commands-v2");
        assert_eq!(payload["bin"], "easybooks");
        assert_eq!(payload["version"], env!("CARGO_PKG_VERSION"));
        let paths: Vec<Vec<String>> = payload["commands"]
            .as_array()
            .expect("commands")
            .iter()
            .map(|item| {
                item["path"]
                    .as_array()
                    .expect("path")
                    .iter()
                    .map(|part| part.as_str().unwrap().to_string())
                    .collect()
            })
            .collect();
        assert!(paths.contains(&vec!["login".into()]));
        assert!(paths.contains(&vec!["invoice".into(), "create".into()]));
        assert!(paths.contains(&vec!["feedback".into(), "create".into()]));
        assert!(paths.contains(&vec!["commands".into()]));
        assert!(paths.contains(&vec!["pair".into(), "join".into()]));
        assert!(paths.contains(&vec!["pair".into(), "status".into()]));
        assert!(paths.contains(&vec!["pair".into(), "snapshot".into()]));
        assert!(paths.contains(&vec!["pair".into(), "inbox".into()]));
        assert!(paths.contains(&vec!["pair".into(), "read".into()]));
        assert!(paths.contains(&vec!["pair".into(), "result".into()]));
        assert!(paths.contains(&vec!["pair".into(), "close".into()]));
        assert!(!paths
            .iter()
            .any(|path| path.first().map(String::as_str) == Some("admin")));
        let cli = Cli::try_parse_from(["easybooks", "commands", "--json"])
            .expect("commands --json should parse");
        assert!(matches!(
            cli.command,
            super::Command::CliCommands { json: true }
        ));
    }

    #[test]
    fn every_public_command_has_a_summary() {
        let payload = super::plugin_cli_commands_payload();
        for command in payload["commands"].as_array().expect("commands") {
            let summary = command["summary"].as_str().unwrap_or_default();
            assert!(
                !summary.trim().is_empty(),
                "public command {} has no summary",
                command["path"]
            );
        }
    }

    #[test]
    fn pair_join_requires_confirm_flag() {
        let cli = Cli::try_parse_from([
            "easybooks",
            "pair",
            "join",
            "--code",
            "ABC234",
            "--user-confirmed",
            "--json",
        ])
        .expect("pair join with confirm should parse");
        match cli.command {
            super::Command::Pair(super::PairCmd::Join {
                code,
                user_confirmed,
                json,
            }) => {
                assert_eq!(code, "ABC234");
                assert!(user_confirmed);
                assert!(json);
            }
            _ => panic!("expected pair join"),
        }
        assert!(Cli::try_parse_from(["easybooks", "pair", "join"]).is_err());
    }
}
