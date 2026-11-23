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
const INSTRUCTIONS: &str = "Scry's view of the merge, rebase, cherry-pick or revert in progress in this repository: start with overview, then conflicts(path), then conflict(path, id), and resolve(path, id, ...) to write a conflict's resolution with a reason. Conflict ids come from the content: they survive other conflicts being resolved. Prefer picking a candidate to writing text. Never stage, commit or continue the operation: the user reviews each resolution in Scry first.";

/// The candidates `resolve` can pick, as their ids.
const PICKS: [&str; 8] = [
    "ours",
    "theirs",
    "ours-then-theirs",
    "theirs-then-ours",
    "neither",
    "base",
    "ours-then-theirs-closed",
    "theirs-then-ours-closed",
];

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
            "prompts/list" => Ok(json!({ "prompts": prompts() })),
            "prompts/get" => prompt(&params),
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
        let answer = match name {
            "overview" => self.tools.overview(),
            "conflicts" => text(&arguments, "path").and_then(|path| self.tools.conflicts(&path)),
            "conflict" => text(&arguments, "path").and_then(|path| {
                let id = text(&arguments, "id")?;
                self.tools.conflict(&path, &id, context(&arguments)?)
            }),
            "resolve" => text(&arguments, "path").and_then(|path| {
                let id = text(&arguments, "id")?;
                let resolution = resolution(&arguments)?;
                let by = self.client.as_deref().unwrap_or("an MCP client");
                self.tools.resolve(&path, &id, resolution, by)
            }),
            _ => {
                return Err((
                    INVALID_PARAMS,
                    format!(
                        "Unknown tool {name}: the tools are overview, conflicts, conflict and resolve"
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
        "capabilities": { "tools": {}, "prompts": {} },
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
                        "enum": PICKS,
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

/// The agent's name and version, as `initialize` gives them.
fn client(params: &Value) -> Option<String> {
    let info = params.get("clientInfo")?;
    let name = info.get("name").and_then(Value::as_str)?.trim();
    if name.is_empty() {
        return None;
    }
    Some(match info.get("version").and_then(Value::as_str) {
        Some(version) if !version.trim().is_empty() => format!("{name} {}", version.trim()),
        _ => name.to_string(),
    })
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
                    PICKS.join(", ")
                )
            })?)
        }
        (None, Some(own)) => {
            let own = own
                .as_str()
                .ok_or_else(|| String::from("text is a string: the resolved lines."))?;
            if own.len() > tools::MAX_TEXT {
                return Err(format!(
                    "text is too long: {} bytes at most for one conflict.",
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
    let dropped = match resolve {
        Resolve::Pick(_) => None,
        Resolve::Text(_) => {
            let dropped = arguments
                .get("dropped")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or_default();
            if dropped.is_empty() || dropped.chars().count() > tools::MAX_DROPPED {
                return Err(format!(
                    "Nothing written: with text, dropped is required, at most {} characters: what of ours or theirs is not in the text, or \"nothing\".",
                    tools::MAX_DROPPED
                ));
            }
            Some(dropped.to_string())
        }
    };
    Ok(Resolution {
        resolve,
        reason: reason.to_string(),
        dropped,
    })
}

/// The prompts, in a stable order.
fn prompts() -> Value {
    json!([{
        "name": "resolve",
        "title": "Resolve the conflicts with Scry",
        "description": "Resolve the conflicts of the operation in progress with Scry's tools, for the user to review in Scry.",
        "arguments": [{
            "name": "path",
            "description": "Only this file, from the repository root.",
            "required": false,
        }],
    }])
}

/// A prompt's messages.
fn prompt(params: &Value) -> Result<Value, (i64, String)> {
    match params.get("name").and_then(Value::as_str) {
        Some("resolve") => {}
        Some(name) => {
            return Err((
                INVALID_PARAMS,
                format!("Unknown prompt {name}: the prompt is resolve"),
            ));
        }
        None => {
            return Err((
                INVALID_PARAMS,
                String::from("prompts/get needs a prompt name"),
            ));
        }
    }
    let path = params
        .get("arguments")
        .and_then(|arguments| arguments.get("path"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty());
    Ok(json!({
        "description": "Resolve the conflicts with Scry",
        "messages": [{
            "role": "user",
            "content": { "type": "text", "text": resolve_prompt(path) },
        }],
    }))
}

/// What the `resolve` prompt asks, for one file or all of them.
fn resolve_prompt(path: Option<&str>) -> String {
    let scope = match path {
        Some(path) => format!("the conflicts of {path}"),
        None => String::from("the conflicts of the operation in progress"),
    };
    format!(
        "Resolve {scope} with Scry's tools (the scry MCP server).

1. Call overview, then conflicts(path) for each file to resolve, then conflict(path, id) for each open conflict: it shows ours, base and theirs, the commits behind each side and the candidates.
2. Resolve each open conflict with resolve. Prefer pick, a candidate Scry offers (ours, theirs, ours-then-theirs, base...), when one keeps what both sides meant. Write text only when none does, and then say in dropped what of ours or theirs is not in it, or \"nothing\".
3. Give each a reason the user can check quickly: which intent of each side is kept and why, in 1 to 3 sentences, naming each side by its label from overview.
4. If resolve answers that the file does not parse, fix that conflict with resolve again.
5. Do not edit the conflicted files yourself, and do not stage, commit, continue, skip or abort the operation: the user reviews each resolution in Scry first.
6. End with a short summary of what you resolved and what you were unsure about, and ask the user to review it with `git scry`."
    )
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
