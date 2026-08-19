use crate::{indexer, query, store};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::PathBuf;

pub fn serve() -> Result<()> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = serde_json::from_str(&line).context("invalid JSON-RPC request")?;
        if request.get("id").is_none() {
            continue;
        }
        let response = dispatch(&request);
        serde_json::to_writer(&mut stdout, &response)?;
        stdout.write_all(b"\n")?;
        stdout.flush()?;
    }
    Ok(())
}

fn dispatch(request: &Value) -> Value {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let result = match request.get("method").and_then(Value::as_str).unwrap_or("") {
        "initialize" => Ok(json!({
            "protocolVersion": "2025-03-26",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "connectome", "version": env!("CARGO_PKG_VERSION")},
            "instructions": "For supported-language navigation, prefer Connectome: use get_overview or search_symbols for discovery, get_symbol only for a required source range, and trace_calls for bounded call paths. Trace rows include caller and callee definition locations plus the call-site line, so use them directly as evidence. Query tools are read-only. Use shell search only when a semantic query cannot answer the question."
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tools()})),
        "tools/call" => call_tool(request.get("params").unwrap_or(&Value::Null)),
        method => Err(anyhow::anyhow!("unknown method: {method}")),
    };
    match result {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err(error) => {
            json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32603, "message": error.to_string()}})
        }
    }
}

fn call_tool(params: &Value) -> Result<Value> {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let value = match name {
        "index_repository" => {
            let root = required_path(&args, "path")?;
            let options = indexer::BuildOptions {
                jdtls_command: args
                    .get("jdtls_command")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                clojure_lsp_command: args
                    .get("clojure_lsp_command")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                typescript_lsp_command: args
                    .get("typescript_lsp_command")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                rust_analyzer_command: args
                    .get("rust_analyzer_command")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                dart_lsp_command: args
                    .get("dart_lsp_command")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                lsp_timeout_ms: args
                    .get("lsp_timeout_ms")
                    .and_then(Value::as_u64)
                    .unwrap_or(5000),
                lsp_mode: crate::lsp::parse_mode(
                    args.get("lsp_mode")
                        .and_then(Value::as_str)
                        .unwrap_or("auto"),
                )?,
            };
            let index = indexer::build_with_options(&root, &options)?;
            let path = store::default_path(&root);
            store::save(&index, &path)?;
            json!({"files": index.files.len(), "symbols": index.symbols.len(), "calls": index.calls.len(), "parsed": index.parsed_files, "reused": index.reused_files, "lsp_resolved": index.lsp_resolved, "lsp_call_hierarchy": index.lsp_call_hierarchy, "lsp_servers": index.lsp_servers, "lsp_capabilities": index.lsp_capabilities, "lsp_warnings": index.lsp_warnings, "lsp_mode": index.lsp_mode, "ms": index.elapsed_ms, "index": path})
        }
        "search_symbols" => {
            let index = load_index(&args)?;
            query::search(
                &index,
                required_str(&args, "query")?,
                args.get("kind").and_then(Value::as_str),
                limit(&args, 20),
            )
        }
        "get_symbol" => {
            let index = load_index(&args)?;
            query::symbol(
                &index,
                required_str(&args, "name")?,
                args.get("context").and_then(Value::as_u64).unwrap_or(0) as usize,
            )?
        }
        "trace_calls" => {
            let index = load_index(&args)?;
            query::trace(
                &index,
                required_str(&args, "name")?,
                args.get("direction")
                    .and_then(Value::as_str)
                    .unwrap_or("outbound"),
                args.get("depth")
                    .and_then(Value::as_u64)
                    .unwrap_or(2)
                    .min(4) as usize,
                args.get("limit")
                    .and_then(Value::as_u64)
                    .unwrap_or(25)
                    .clamp(1, 25) as usize,
            )?
        }
        "get_overview" => query::overview(&load_index(&args)?),
        _ => return Err(anyhow::anyhow!("unknown tool: {name}")),
    };
    Ok(json!({"content": [{"type": "text", "text": serde_json::to_string(&value)?}]}))
}

fn load_index(args: &Value) -> Result<crate::model::Index> {
    let root = args
        .get("path")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir()?);
    store::load(&store::default_path(&root))
}

fn required_path(args: &Value, name: &str) -> Result<PathBuf> {
    Ok(PathBuf::from(required_str(args, name)?))
}
fn required_str<'a>(args: &'a Value, name: &str) -> Result<&'a str> {
    args.get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("missing string argument: {name}"))
}
fn limit(args: &Value, default: usize) -> usize {
    args.get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(default as u64)
        .clamp(1, 200) as usize
}

fn tools() -> Vec<Value> {
    vec![
        tool(
            "index_repository",
            "Index Java, Clojure, JavaScript, TypeScript, Rust, and Dart sources into a compact local graph",
            json!({"type":"object","properties":{"path":{"type":"string"},"jdtls_command":{"type":"string"},"clojure_lsp_command":{"type":"string"},"typescript_lsp_command":{"type":"string","description":"Optional command, for example typescript-language-server --stdio"},"rust_analyzer_command":{"type":"string","description":"Optional command, for example rust-analyzer"},"dart_lsp_command":{"type":"string","description":"Optional command, for example dart language-server --protocol=lsp"},"lsp_mode":{"type":"string","enum":["auto","on","off"],"default":"auto"},"lsp_timeout_ms":{"type":"integer","default":5000}},"required":["path"]}),
        ),
        tool(
            "search_symbols",
            "Find symbols; returns compact signatures and locations",
            query_schema(
                json!({"query":{"type":"string"},"kind":{"type":"string"},"limit":{"type":"integer","maximum":200}}),
                vec!["query"],
            ),
        ),
        tool(
            "get_symbol",
            "Read exactly one symbol body by qualified name",
            query_schema(
                json!({"name":{"type":"string"},"context":{"type":"integer","maximum":20}}),
                vec!["name"],
            ),
        ),
        tool(
            "trace_calls",
            "Trace a small bounded inbound or outbound resolved call path. Each edge includes caller/callee selectors, definition locations (`from_at`/`to_at`), and call-site `line`; use these directly as evidence. Use the exact line-qualified `name` returned by search_symbols to select an overload; prefer this over reading multiple symbol bodies",
            query_schema(
                json!({"name":{"type":"string"},"direction":{"enum":["inbound","outbound"]},"depth":{"type":"integer","maximum":4,"default":2},"limit":{"type":"integer","maximum":25,"default":25}}),
                vec!["name"],
            ),
        ),
        tool(
            "get_overview",
            "Get a small repository architecture summary",
            query_schema(json!({}), vec![]),
        ),
    ]
}

fn tool(name: &str, description: &str, input_schema: Value) -> Value {
    let read_only = name != "index_repository";
    json!({
        "name":name,
        "description":description,
        "inputSchema":input_schema,
        "annotations": {"readOnlyHint":read_only,"destructiveHint":false,"idempotentHint":true}
    })
}
fn query_schema(extra: Value, required: Vec<&str>) -> Value {
    let mut properties = serde_json::Map::new();
    properties.insert(
        "path".into(),
        json!({"type":"string","description":"Repository root; defaults to current directory"}),
    );
    if let Value::Object(extra) = extra {
        properties.extend(extra);
    }
    json!({"type":"object","properties":properties,"required":required})
}
