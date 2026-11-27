//! The Model Context Protocol, by hand (vault decision: no SDK for version
//! 1): JSON-RPC 2.0 messages, one per line on standard input and output,
//! and the few methods a server offering tools answers. Moving to an SDK
//! later replaces this module only; the tools stay plain functions.

use scry_core::git::GitCli;
use serde_json::{Value, json};

use scry_core::resolution::CandidateKind;

use crate::tools::{self, Resolution, Resolve, Tools};

/// The protocol versions this server speaks, newest first: it uses
/// nothing a later one changed.
const PROTOCOL_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

// JSON-RPC's error codes.
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// What the server tells the agent about itself when it connects.
const INSTRUCTIONS: &str = "Scry's view of the merge, rebase, cherry-pick or revert in progress in this repository: start with overview, then conflicts(path), then conflict(path, id), and resolve(path, id, ...) to write a conflict's resolution with a reason. When git scry wait prints \"Scry asks: explain <path> <id>\", read that conflict and answer with explain(path, id, text), or explain(path, id, text, failed: true) saying why you could not. When the id starts with L, the ask is about lines the user selected in Scry's Changes window, with or without an operation in progress: read them with lines(path, id) and answer with explain the same way. When it prints \"Scry asks: resolve <path> <id>\", read that conflict and propose a resolution with suggest(path, id, pick or text, reason): nothing is written, the user accepts it in Scry or not; or suggest(path, id, reason, failed: true) saying why you could not. Conflict ids come from the content: they survive other conflicts being resolved. Prefer picking a candidate to writing text. Never stage, commit or continue the operation: the user reviews each resolution in Scry first.";

/// The candidates `resolve` can pick, as their ids.
fn picks() -> Vec<&'static str> {
    CandidateKind::ALL.iter().map(|kind| kind.id()).collect()
}

/// One connection: an agent's messages in, answers out.
pub struct Server {
    tools: Tools,
    /// The agent's name and version, from `initialize`: who `resolve`
    /// records, as the agent says.
    client: Option<String>,
}

impl Server {
    /// A server running git as `git` does: from the folder the agent
    /// started it in.
    pub fn new(git: GitCli) -> Server {
        Server {
            tools: Tools::new(git),
            client: None,
        }
    }

    /// The answer to one message, one line of input; `None` for a
    /// notification, which gets none.
    pub fn answer(&mut self, line: &str) -> Option<Value> {
        let Ok(message) = serde_json::from_str::<Value>(line) else {
            return Some(error(Value::Null, PARSE_ERROR, "Not JSON"));
        };
        let id = message.get("id").cloned();
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            // An answer to a request: this server sends none.
            if message.get("result").is_some() || message.get("error").is_some() {
                return None;
            }
            return Some(error(
                id.unwrap_or(Value::Null),
                INVALID_REQUEST,
                "Not a JSON-RPC request: no method",
            ));
        };
        // A notification (`notifications/initialized`, a cancellation):
        // nothing to answer.
        let id = id?;
        let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
        let result = match method {
            "initialize" => {
                self.client = client(&params);
                Ok(initialize(&params))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": definitions() })),
            "tools/call" => self.call(&params),
            _ => Err((METHOD_NOT_FOUND, format!("Unknown method {method}"))),
        };
        Some(match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err((code, message)) => error(id, code, &message),
        })
    }

    /// Runs a tool. A call the agent should correct (a wrong path, a stale
    /// id, an argument missing) is a result marked as an error, which the
    /// agent reads; an unknown tool is a protocol error.
    fn call(&mut self, params: &Value) -> Result<Value, (i64, String)> {
        let Some(name) = params.get("name").and_then(Value::as_str) else {
            return Err((INVALID_PARAMS, String::from("tools/call needs a tool name")));
        };
        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        if let Some(unknown) = unknown_argument(name, &arguments) {
            return Ok(json!({
                "content": [{ "type": "text", "text": unknown }],
                "isError": true,
            }));
        }
        let answer = match name {
            "overview" => self.tools.overview(),
            "conflicts" => text(&arguments, "path").and_then(|path| self.tools.conflicts(&path)),
            "conflict" => text(&arguments, "path").and_then(|path| {
                let id = text(&arguments, "id")?;
                self.tools.conflict(&path, &id, context(&arguments)?)
            }),
            "lines" => text(&arguments, "path").and_then(|path| {
                let id = text(&arguments, "id")?;
                self.tools.lines(&path, &id)
            }),
            "resolve" => text(&arguments, "path").and_then(|path| {
                let id = text(&arguments, "id")?;
                let resolution = resolution(&arguments)?;
                let by = self.client.as_deref().unwrap_or("an MCP client");
                self.tools.resolve(&path, &id, resolution, by)
            }),
            "explain" => text(&arguments, "path").and_then(|path| {
                let id = text(&arguments, "id")?;
                let (explanation, failed) = explanation(&arguments)?;
                let by = self.client.as_deref().unwrap_or("an MCP client");
                self.tools.explain(&path, &id, &explanation, failed, by)
            }),
            "suggest" => text(&arguments, "path").and_then(|path| {
                let id = text(&arguments, "id")?;
                let suggestion = suggestion(&arguments)?;
                let by = self.client.as_deref().unwrap_or("an MCP client");
                self.tools.suggest(&path, &id, suggestion, by)
            }),
            _ => {
                return Err((
                    INVALID_PARAMS,
                    format!(
                        "Unknown tool {name}: the tools are overview, conflicts, conflict, lines, resolve, explain and suggest"
                    ),
                ));
            }
        };
        let (text, is_error) = match answer {
            Ok(text) => (text, false),
            Err(text) => (text, true),
        };
        Ok(json!({
            "content": [{ "type": "text", "text": text }],
            "isError": is_error,
        }))
    }
}

/// The answer to `initialize`: the client's protocol version when this
/// server speaks it, else the newest it speaks, for the client to decide.
fn initialize(params: &Value) -> Value {
    let asked = params.get("protocolVersion").and_then(Value::as_str);
    let version = PROTOCOL_VERSIONS
        .into_iter()
        .find(|version| Some(*version) == asked)
        .unwrap_or(PROTOCOL_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "git-scry", "version": env!("CARGO_PKG_VERSION") },
        "instructions": INSTRUCTIONS,
    })
}

/// The tools, in a stable order, each described in a line.
fn definitions() -> Value {
    let read_only = json!({ "readOnlyHint": true, "openWorldHint": false });
    let path = json!({
        "type": "string",
        "description": "The file's path from the repository root, as overview lists it.",
    });
    json!([
        {
            "name": "overview",
            "title": "Operation in progress",
            "description": "The merge, rebase, cherry-pick or revert in progress: who is on each side, and the files left with conflicts, with how many are open. Start here.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": read_only,
        },
        {
            "name": "conflicts",
            "title": "Conflicts of a file",
            "description": "One line per conflict of a file: its id, line, what each side did, the candidates and whether the file parses with each, open or resolved.",
            "inputSchema": {
                "type": "object",
                "properties": { "path": path },
                "required": ["path"],
                "additionalProperties": false,
            },
            "annotations": read_only,
        },
        {
            "name": "conflict",
            "title": "One conflict",
            "description": "One conflict in full: ours, base and theirs with who is on each side and the last commit to change it, the lines around, and each candidate with whether the file parses with it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": path,
                    "id": {
                        "type": "string",
                        "description": "The conflict's id, as conflicts lists it (c1a2b3c4).",
                    },
                    "context": {
                        "type": "integer",
                        "minimum": 0,
                        "maximum": tools::MAX_CONTEXT,
                        "default": tools::DEFAULT_CONTEXT,
                        "description": "Lines shown above and below the conflict.",
                    },
                },
                "required": ["path", "id"],
                "additionalProperties": false,
            },
            "annotations": read_only,
        },
        {
            "name": "resolve",
            "title": "Resolve a conflict",
            "description": "Writes one conflict's resolution in the file, that conflict's lines only, and keeps your reason for the user's review in Scry. Pick a candidate when one keeps what both sides meant; write text only when none does. Answers whether the file parses. Stages nothing.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": path,
                    "id": {
                        "type": "string",
                        "description": "The conflict's id, as conflicts lists it (c1a2b3c4).",
                    },
                    "pick": {
                        "type": "string",
                        "enum": picks(),
                        "description": "A candidate the conflict offers, as conflicts lists them. Either pick or text.",
                    },
                    "text": {
                        "type": "string",
                        "maxLength": tools::MAX_TEXT,
                        "description": "The conflict's resolved lines, written by you, without markers; empty removes them. Either pick or text.",
                    },
                    "reason": {
                        "type": "string",
                        "minLength": tools::REASON_LENGTH.start(),
                        "maxLength": tools::REASON_LENGTH.end(),
                        "description": "For the user's review: which intent of each side is kept and why, in 1 to 3 sentences, naming each side by its label from overview. Not \"resolved the conflict\".",
                    },
                    "dropped": {
                        "type": "string",
                        "maxLength": tools::MAX_DROPPED,
                        "description": "With text: what of ours or theirs is not in it, or \"nothing\".",
                    },
                    "replace": {
                        "type": "boolean",
                        "default": false,
                        "description": "Replace a resolution made by someone else (by hand, another tool) or reviewed by the user. Only when the user asked for it.",
                    },
                },
                "required": ["path", "id", "reason"],
                "additionalProperties": false,
            },
            "annotations": {
                "readOnlyHint": false,
                "destructiveHint": true,
                "idempotentHint": true,
                "openWorldHint": false,
            },
        },
        {
            "name": "lines",
            "title": "Selected lines",
            "description": "The lines the user selected in Scry's Changes window and asked about (git scry wait prints \"Scry asks: explain <path> <id>\" with an id starting with L): the change around them as Scry shows it and who last wrote them. Works with no merge in progress. Answer with explain.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": path,
                    "id": {
                        "type": "string",
                        "description": "The ask's id, as git scry wait printed it (L1a2b3c4d).",
                    },
                },
                "required": ["path", "id"],
                "additionalProperties": false,
            },
            "annotations": read_only,
        },
        {
            "name": "explain",
            "title": "Explain a conflict or selected lines",
            "description": "Answers an ask the user made in Scry (git scry wait prints \"Scry asks: explain <path> <id>\"). For a conflict: what each side meant to do, the risk of each candidate, which one looks safe and why. For selected lines (an id starting with L, read with lines): what they do and why they changed. Scry shows it beside the conflict or the lines. Writes nothing in the repository's files.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": path,
                    "id": {
                        "type": "string",
                        "description": "The conflict's id, or the selected lines' (L1a2b3c4d), as the ask names it.",
                    },
                    "text": {
                        "type": "string",
                        "minLength": tools::FAILURE_LENGTH.start(),
                        "maxLength": tools::EXPLANATION_LENGTH.end(),
                        "description": "2 to 4 sentences for the user (20 characters at least), naming each side of a conflict by its label from overview; with failed, why you could not explain. Calling again replaces it.",
                    },
                    "failed": {
                        "type": "boolean",
                        "description": "true when you could not explain the conflict (it is gone, it could not be read): Scry shows text as the problem you met, and the user asks again.",
                    },
                },
                "required": ["path", "id", "text"],
                "additionalProperties": false,
            },
            "annotations": {
                "readOnlyHint": false,
                "destructiveHint": false,
                "idempotentHint": true,
                "openWorldHint": false,
            },
        },
        {
            "name": "suggest",
            "title": "Propose a resolution",
            "description": "Answers an ask the user made in Scry (git scry wait prints \"Scry asks: resolve <path> <id>\"): proposes one conflict's resolution with a reason. Nothing is written to the file: Scry shows the proposal beside the candidates, and the user accepts, edits or rejects it. Pick a candidate when one keeps what both sides meant; write text only when none does. Answers whether the file parses with it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": path,
                    "id": {
                        "type": "string",
                        "description": "The conflict's id, or the selected lines' (L1a2b3c4d), as the ask names it.",
                    },
                    "pick": {
                        "type": "string",
                        "enum": picks(),
                        "description": "A candidate the conflict offers, as conflicts lists them. Either pick or text.",
                    },
                    "text": {
                        "type": "string",
                        "maxLength": tools::MAX_TEXT,
                        "description": "The conflict's resolved lines, written by you, without markers; empty removes them. Either pick or text.",
                    },
                    "reason": {
                        "type": "string",
                        "minLength": tools::FAILURE_LENGTH.start(),
                        "maxLength": tools::REASON_LENGTH.end(),
                        "description": "For the user: which intent of each side is kept and why, in 1 to 3 sentences (20 characters at least), naming each side by its label from overview; with failed, why you could not propose one. Calling again replaces it.",
                    },
                    "dropped": {
                        "type": "string",
                        "maxLength": tools::MAX_DROPPED,
                        "description": "With text: what of ours or theirs is not in it, or \"nothing\".",
                    },
                    "failed": {
                        "type": "boolean",
                        "description": "true when you could not propose a resolution (the conflict is gone, it could not be read), without pick or text: Scry shows reason as the problem you met.",
                    },
                },
                "required": ["path", "id", "reason"],
                "additionalProperties": false,
            },
            "annotations": {
                "readOnlyHint": false,
                "destructiveHint": false,
                "idempotentHint": true,
                "openWorldHint": false,
            },
        },
    ])
}

/// The longest agent name kept, in characters.
const MAX_CLIENT: usize = 80;

/// An argument the tool's schema does not have (`additionalProperties:
/// false`), with the ones it has: a misspelt `replace` must not be ignored.
fn unknown_argument(name: &str, arguments: &Value) -> Option<String> {
    let definitions = definitions();
    let properties = definitions
        .as_array()?
        .iter()
        .find(|tool| tool["name"] == name)?["inputSchema"]["properties"]
        .as_object()?
        .clone();
    let unknown = arguments
        .as_object()?
        .keys()
        .find(|key| !properties.contains_key(*key))?;
    let known: Vec<&str> = properties.keys().map(String::as_str).collect();
    Some(format!(
        "{name} has no argument {unknown}. Its arguments: {}.",
        known.join(", ")
    ))
}

/// The agent's name and version, as `initialize` gives them, on one line of
/// printable characters, at most `MAX_CLIENT`: the app shows it as data.
fn client(params: &Value) -> Option<String> {
    let info = params.get("clientInfo")?;
    let field = |name: &str| {
        info.get(name)
            .and_then(Value::as_str)
            .map(one_line)
            .unwrap_or_default()
    };
    let name = field("name");
    if name.is_empty() {
        return None;
    }
    let version = field("version");
    let client = match version.is_empty() {
        true => name,
        false => format!("{name} {version}"),
    };
    Some(
        client
            .chars()
            .take(MAX_CLIENT)
            .collect::<String>()
            .trim_end()
            .to_string(),
    )
}

/// `text` with control characters and runs of spaces as one space.
fn one_line(text: &str) -> String {
    text.split(|character: char| character.is_control() || character.is_whitespace())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// `explain`'s text, its length checked, and whether it says why the agent
/// could not explain.
fn explanation(arguments: &Value) -> Result<(String, bool), String> {
    let text = text(arguments, "text")?;
    let failed = match arguments.get("failed") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(failed)) => *failed,
        Some(other) => return Err(format!("failed must be true or false, not {other}.")),
    };
    let (what, range) = match failed {
        true => ("The reason", tools::FAILURE_LENGTH),
        false => ("The explanation", tools::EXPLANATION_LENGTH),
    };
    let length = text.trim().chars().count();
    if !range.contains(&length) {
        return Err(format!(
            "{what} has {length} characters: give {} to {}.",
            range.start(),
            range.end()
        ));
    }
    Ok((text, failed))
}

/// `suggest`'s resolution (`resolve`'s arguments without `replace`), or
/// with `failed` the reason the agent could not propose one.
fn suggestion(arguments: &Value) -> Result<Result<Resolution, String>, String> {
    match arguments.get("failed") {
        None | Some(Value::Null) | Some(Value::Bool(false)) => resolution(arguments).map(Ok),
        Some(Value::Bool(true)) => {
            if ["pick", "text"]
                .iter()
                .any(|name| arguments.get(*name).is_some_and(|value| !value.is_null()))
            {
                return Err(String::from(
                    "With failed, give only the reason: no pick or text.",
                ));
            }
            let reason = arguments
                .get("reason")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or_default();
            let length = reason.chars().count();
            if !tools::FAILURE_LENGTH.contains(&length) {
                return Err(format!(
                    "The reason has {length} characters: give {} to {}.",
                    tools::FAILURE_LENGTH.start(),
                    tools::FAILURE_LENGTH.end()
                ));
            }
            Ok(Err(reason.to_string()))
        }
        Some(other) => Err(format!("failed must be true or false, not {other}.")),
    }
}

/// `resolve`'s pick or text, reason and, with a text, what it leaves out.
fn resolution(arguments: &Value) -> Result<Resolution, String> {
    let pick = arguments.get("pick").filter(|value| !value.is_null());
    let own = arguments.get("text").filter(|value| !value.is_null());
    let resolve = match (pick, own) {
        (Some(_), Some(_)) => return Err(String::from("Give either pick or text, not both.")),
        (None, None) => {
            return Err(String::from(
                "Give pick (a candidate, as conflicts lists them) or text (the resolved lines).",
            ));
        }
        (Some(pick), None) => {
            let id = pick.as_str().unwrap_or_default();
            Resolve::Pick(CandidateKind::from_id(id).ok_or_else(|| {
                format!(
                    "pick {pick} is not a candidate: one of {}.",
                    picks().join(", ")
                )
            })?)
        }
        (None, Some(own)) => {
            let own = own
                .as_str()
                .ok_or_else(|| String::from("text is a string: the resolved lines."))?;
            if own.chars().count() > tools::MAX_TEXT {
                return Err(format!(
                    "text is too long: {} characters at most for one conflict.",
                    tools::MAX_TEXT
                ));
            }
            Resolve::Text(own.to_string())
        }
    };
    let reason = arguments
        .get("reason")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if !tools::REASON_LENGTH.contains(&reason.chars().count()) {
        return Err(format!(
            "Nothing written: reason is required, {} to {} characters: which intent of each side is kept and why, naming each side by its label.",
            tools::REASON_LENGTH.start(),
            tools::REASON_LENGTH.end()
        ));
    }
    // Required with text; with a pick, kept when given, as the agent's note.
    let dropped = arguments
        .get("dropped")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|dropped| !dropped.is_empty());
    let required = matches!(resolve, Resolve::Text(_));
    if dropped.is_none() && required
        || dropped.is_some_and(|dropped| dropped.chars().count() > tools::MAX_DROPPED)
    {
        return Err(format!(
            "Nothing written: with text, dropped is required, at most {} characters: what of ours or theirs is not in the text, or \"nothing\".",
            tools::MAX_DROPPED
        ));
    }
    let dropped = dropped.map(str::to_string);
    let replace = match arguments.get("replace") {
        None | Some(Value::Null) => false,
        Some(value) => value
            .as_bool()
            .ok_or_else(|| String::from("replace is true or false."))?,
    };
    Ok(Resolution {
        resolve,
        replace,
        reason: reason.to_string(),
        dropped,
    })
}

/// A text argument, required.
fn text(arguments: &Value, name: &str) -> Result<String, String> {
    match arguments.get(name).and_then(Value::as_str) {
        Some(value) if !value.is_empty() => Ok(value.to_string()),
        _ => Err(format!("Missing {name}: a string is required.")),
    }
}

/// `context`, lines around a conflict: a whole number up to the maximum.
fn context(arguments: &Value) -> Result<usize, String> {
    match arguments.get("context") {
        None | Some(Value::Null) => Ok(tools::DEFAULT_CONTEXT),
        Some(value) => value
            .as_u64()
            .and_then(|lines| usize::try_from(lines).ok())
            .filter(|lines| *lines <= tools::MAX_CONTEXT)
            .ok_or_else(|| {
                format!(
                    "context is a number of lines, from 0 to {}.",
                    tools::MAX_CONTEXT
                )
            }),
    }
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}
