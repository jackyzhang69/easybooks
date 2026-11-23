use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};
use jz_plugin_lifecycle::commands::{
    clap_payload, from_clap, payload, ArgDoc, ArgKind, CommandDoc, COMMANDS_SCHEMA,
};
use serde_json::json;

#[derive(Clone, Copy, ValueEnum)]
enum Mode {
    Fast,
    Careful,
}

#[derive(Parser)]
#[command(name = "demo")]
struct Cli {
    /// Print more detail.
    #[arg(long, global = true)]
    verbose: bool,
    #[command(subcommand)]
    command: Top,
}

#[derive(Subcommand)]
enum Top {
    /// Read one record.
    ///
    /// Returns the record summary without credentials.
    Show {
        /// Record id.
        id: String,
        /// How to read.
        #[arg(long, value_enum, required = true)]
        mode: Mode,
        /// Output file.
        #[arg(short = 'o')]
        out: Option<String>,
        /// Dry run.
        #[arg(long)]
        dry_run: bool,
    },
    /// Work with accounts.
    #[command(subcommand)]
    Account(AccountCommand),
    /// Hidden helper.
    #[command(hide = true)]
    Secret,
    /// Operator-only surface.
    Admin(AdminArgs),
}

#[derive(Subcommand)]
enum AccountCommand {
    /// List accounts.
    List,
    /// Remove an account.
    Remove {
        /// Account name.
        name: String,
    },
    /// Internal plumbing.
    #[command(hide = true)]
    Rebuild,
}

#[derive(Args)]
struct AdminArgs {
    #[arg(long)]
    force: bool,
}

#[test]
fn clap_payload_lists_only_public_leaves() {
    let value = clap_payload(Cli::command(), "demo", "1.2.3");
    assert_eq!(value["schema"], COMMANDS_SCHEMA);
    assert_eq!(value["bin"], "demo");
    assert_eq!(value["version"], "1.2.3");
    let paths: Vec<Vec<&str>> = value["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            c["path"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| p.as_str().unwrap())
                .collect()
        })
        .collect();
    assert_eq!(
        paths,
        vec![
            vec!["show"],
            vec!["account", "list"],
            vec!["account", "remove"]
        ]
    );
}

#[test]
fn kinds_required_values_and_global_flag() {
    let value = clap_payload(Cli::command(), "demo", "1.2.3");
    assert_eq!(
        value["global_args"],
        json!([{"name": "--verbose", "kind": "flag", "required": false, "help": "Print more detail"}])
    );
    let show = &value["commands"][0];
    // clap derive strips the trailing period of a one-line `about`; `detail` keeps the full text.
    assert_eq!(show["summary"], "Read one record");
    assert_eq!(
        show["detail"],
        "Read one record.\n\nReturns the record summary without credentials."
    );
    let args = show["args"].as_array().unwrap();
    let by_name = |name: &str| {
        args.iter()
            .find(|a| a["name"] == name)
            .unwrap_or_else(|| panic!("no {name}"))
    };
    assert_eq!(by_name("<ID>")["kind"], "positional");
    assert_eq!(by_name("<ID>")["required"], true);
    assert_eq!(by_name("--mode")["kind"], "option");
    assert_eq!(by_name("--mode")["required"], true);
    assert_eq!(by_name("--mode")["values"], json!(["fast", "careful"]));
    assert_eq!(by_name("-o")["kind"], "option");
    assert_eq!(by_name("-o")["required"], false);
    assert_eq!(by_name("--dry-run")["kind"], "flag");
    assert!(by_name("--dry-run").get("values").is_none());
    // The global flag appears only under global_args.
    assert!(args.iter().all(|a| a["name"] != "--verbose"));
    // Help and version are never listed.
    assert!(args
        .iter()
        .all(|a| a["name"] != "--help" && a["name"] != "--version"));
    // A command with no long description omits `detail`.
    assert!(value["commands"][1].get("detail").is_none());
}

#[test]
fn from_clap_matches_payload_inputs() {
    let (globals, commands) = from_clap(Cli::command());
    assert_eq!(globals.len(), 1);
    assert_eq!(commands.len(), 3);
    assert_eq!(commands[2].args[0].name, "<NAME>");
}

#[test]
fn manual_payload_omits_empty_optional_fields() {
    let doc = CommandDoc {
        path: vec!["run".into()],
        summary: "Run it.".into(),
        detail: None,
        args: vec![ArgDoc {
            name: "--fast".into(),
            kind: ArgKind::Flag,
            required: false,
            values: vec![],
            help: Some(String::new()),
        }],
    };
    let value = payload("demo", "0.1.0", &[], &[doc]);
    assert_eq!(
        value,
        json!({
            "schema": "plugin-cli-commands-v2",
            "bin": "demo",
            "version": "0.1.0",
            "global_args": [],
            "commands": [{"path": ["run"], "summary": "Run it.", "args": [{"name": "--fast", "kind": "flag", "required": false}]}],
        })
    );
}
