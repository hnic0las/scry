//! The Model Context Protocol, by hand (vault decision: no SDK for version
//! 1): JSON-RPC 2.0 messages, one per line on standard input and output,
//! and the few methods a server offering tools answers. Moving to an SDK
//! later replaces this module only; the tools stay plain functions.

use scry_core::git::GitCli;
use serde_json::{Value, json};

use crate::tools::{self, Tools};

/// The protocol versions this server speaks, newest first: it uses
/// nothing a later one changed.
const PROTOCOL_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

// JSON-RPC's error codes.
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// What the server tells the agent about itself when it connects.
const INSTRUCTIONS: &str = "Scry's view of the merge, rebase, cherry-pick or revert in progress in this repository: start with overview, then conflicts(path), then conflict(path, id). Conflict ids come from the content: they survive other conflicts being resolved. These tools only read; staging and continuing are the user's, in Scry.";

/// One connection: an agent's messages in, answers out.
pub struct Server {
    tools: Tools,
}

impl Server {
    /// A server running git as `git` does: from the folder the agent
    /// started it in.
    pub fn new(git: GitCli) -> Server {
        Server {
            tools: Tools::new(git),
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
            "initialize" => Ok(initialize(&params)),
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
        let answer = match name {
            "overview" => self.tools.overview(),
            "conflicts" => text(&arguments, "path").and_then(|path| self.tools.conflicts(&path)),
            "conflict" => text(&arguments, "path").and_then(|path| {
                let id = text(&arguments, "id")?;
                self.tools.conflict(&path, &id, context(&arguments)?)
            }),
            _ => {
                return Err((
                    INVALID_PARAMS,
                    format!("Unknown tool {name}: the tools are overview, conflicts and conflict"),
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
    ])
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
