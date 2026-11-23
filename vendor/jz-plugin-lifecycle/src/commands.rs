//! `commands --json` payload, schema `plugin-cli-commands-v2`.
//!
//! The payload is the single source for the generated
//! `references/command-surface.md` and for `assert-skill-surface.py`.

use serde_json::{Map, Value};

pub const COMMANDS_SCHEMA: &str = "plugin-cli-commands-v2";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArgKind {
    Option,
    Flag,
    Positional,
}

impl ArgKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ArgKind::Option => "option",
            ArgKind::Flag => "flag",
            ArgKind::Positional => "positional",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArgDoc {
    pub name: String,
    pub kind: ArgKind,
    pub required: bool,
    pub values: Vec<String>,
    pub help: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandDoc {
    pub path: Vec<String>,
    pub summary: String,
    pub detail: Option<String>,
    pub args: Vec<ArgDoc>,
}

fn arg_value(arg: &ArgDoc) -> Value {
    let mut map = Map::new();
    map.insert("name".into(), Value::String(arg.name.clone()));
    map.insert("kind".into(), Value::String(arg.kind.as_str().into()));
    map.insert("required".into(), Value::Bool(arg.required));
    if !arg.values.is_empty() {
        map.insert(
            "values".into(),
            Value::Array(arg.values.iter().cloned().map(Value::String).collect()),
        );
    }
    if let Some(help) = arg.help.as_deref().filter(|help| !help.is_empty()) {
        map.insert("help".into(), Value::String(help.into()));
    }
    Value::Object(map)
}

fn command_value(command: &CommandDoc) -> Value {
    let mut map = Map::new();
    map.insert(
        "path".into(),
        Value::Array(command.path.iter().cloned().map(Value::String).collect()),
    );
    map.insert("summary".into(), Value::String(command.summary.clone()));
    if let Some(detail) = command
        .detail
        .as_deref()
        .filter(|detail| !detail.is_empty())
    {
        map.insert("detail".into(), Value::String(detail.into()));
    }
    map.insert(
        "args".into(),
        Value::Array(command.args.iter().map(arg_value).collect()),
    );
    Value::Object(map)
}

/// Build the `plugin-cli-commands-v2` document.
pub fn payload(bin: &str, version: &str, global_args: &[ArgDoc], commands: &[CommandDoc]) -> Value {
    let mut map = Map::new();
    map.insert("schema".into(), Value::String(COMMANDS_SCHEMA.into()));
    map.insert("bin".into(), Value::String(bin.into()));
    map.insert("version".into(), Value::String(version.into()));
    map.insert(
        "global_args".into(),
        Value::Array(global_args.iter().map(arg_value).collect()),
    );
    map.insert(
        "commands".into(),
        Value::Array(commands.iter().map(command_value).collect()),
    );
    Value::Object(map)
}

#[cfg(feature = "clap")]
const SKIPPED_COMMANDS: [&str; 4] = ["help", "admin", "debug", "internal"];

#[cfg(feature = "clap")]
fn is_public_command(command: &clap::Command) -> bool {
    !command.is_hide_set() && !SKIPPED_COMMANDS.contains(&command.get_name())
}

#[cfg(feature = "clap")]
fn is_public_arg(arg: &clap::Arg) -> bool {
    !arg.is_hide_set() && arg.get_id() != "help" && arg.get_id() != "version"
}

#[cfg(feature = "clap")]
fn arg_doc(arg: &clap::Arg) -> ArgDoc {
    let name = if let Some(long) = arg.get_long() {
        format!("--{long}")
    } else if let Some(short) = arg.get_short() {
        format!("-{short}")
    } else {
        let value_name = arg
            .get_value_names()
            .and_then(|names| names.first())
            .map(|name| name.to_string())
            .unwrap_or_else(|| arg.get_id().as_str().to_uppercase());
        format!("<{value_name}>")
    };
    let kind = if arg.is_positional() {
        ArgKind::Positional
    } else if arg.get_action().takes_values() {
        ArgKind::Option
    } else {
        ArgKind::Flag
    };
    let values = arg
        .get_possible_values()
        .into_iter()
        .filter(|value| !value.is_hide_set())
        .map(|value| value.get_name().to_string())
        .collect();
    ArgDoc {
        name,
        kind,
        required: arg.is_required_set(),
        values,
        help: arg.get_help().map(|help| help.to_string()),
    }
}

#[cfg(feature = "clap")]
fn command_doc(command: &clap::Command, path: &[String]) -> CommandDoc {
    let summary = command
        .get_about()
        .map(|about| about.to_string())
        .unwrap_or_default();
    let detail = command
        .get_long_about()
        .map(|long| long.to_string())
        .filter(|long| !long.is_empty() && *long != summary);
    let args = command
        .get_arguments()
        .filter(|arg| is_public_arg(arg) && !arg.is_global_set())
        .map(arg_doc)
        .collect();
    CommandDoc {
        path: path.to_vec(),
        summary,
        detail,
        args,
    }
}

#[cfg(feature = "clap")]
fn collect_leaves(command: &clap::Command, path: &mut Vec<String>, out: &mut Vec<CommandDoc>) {
    for sub in command
        .get_subcommands()
        .filter(|sub| is_public_command(sub))
    {
        path.push(sub.get_name().to_string());
        if sub.get_subcommands().any(is_public_command) {
            collect_leaves(sub, path, out);
        } else {
            out.push(command_doc(sub, path));
        }
        path.pop();
    }
}

/// Walk a clap command tree: public leaf commands in declaration order.
/// Hidden commands and the names `help`, `admin`, `debug`, `internal` are
/// skipped together with everything below them.
#[cfg(feature = "clap")]
pub fn from_clap(mut root: clap::Command) -> (Vec<ArgDoc>, Vec<CommandDoc>) {
    root.build();
    let globals = root
        .get_arguments()
        .filter(|arg| is_public_arg(arg) && arg.is_global_set())
        .map(arg_doc)
        .collect();
    let mut commands = Vec::new();
    collect_leaves(&root, &mut Vec::new(), &mut commands);
    (globals, commands)
}

#[cfg(feature = "clap")]
pub fn clap_payload(root: clap::Command, bin: &str, version: &str) -> Value {
    let (globals, commands) = from_clap(root);
    payload(bin, version, &globals, &commands)
}
