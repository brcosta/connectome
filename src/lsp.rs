//! Optional LSP enrichment. The fast path never starts a language server.
use crate::model::{Call, FileRecord, Language, Symbol};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LspMode {
    #[default]
    Auto,
    On,
    Off,
}

impl LspMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::On => "on",
            Self::Off => "off",
        }
    }
}

pub fn parse_mode(value: &str) -> Result<LspMode> {
    match value.to_ascii_lowercase().as_str() {
        "auto" => Ok(LspMode::Auto),
        "on" | "required" => Ok(LspMode::On),
        "off" | "disabled" => Ok(LspMode::Off),
        _ => Err(anyhow::anyhow!(
            "invalid LSP mode '{value}'; expected auto, on, or off"
        )),
    }
}

pub struct ResolveResult {
    pub resolved: u32,
    pub call_hierarchy: u32,
    pub servers: Vec<String>,
    pub capabilities: Vec<String>,
    pub warnings: Vec<String>,
}

pub fn resolve_calls(
    root: &Path,
    files: &[FileRecord],
    symbols: &[Symbol],
    calls: &mut Vec<Call>,
    options: &crate::indexer::BuildOptions,
) -> ResolveResult {
    let mut result = ResolveResult {
        resolved: 0,
        call_hierarchy: 0,
        servers: Vec::new(),
        capabilities: Vec::new(),
        warnings: Vec::new(),
    };
    for (language, command, label) in [
        (
            Language::Java,
            effective_command(
                options.jdtls_command.as_deref(),
                "CONNECTOME_JDTLS_COMMAND",
                "jdtls",
                options.lsp_mode,
            ),
            "jdtls",
        ),
        (
            Language::Clojure,
            effective_command(
                options.clojure_lsp_command.as_deref(),
                "CONNECTOME_CLOJURE_LSP_COMMAND",
                "clojure-lsp listen",
                options.lsp_mode,
            ),
            "clojure-lsp",
        ),
    ] {
        let Some(command) = command else {
            if options.lsp_mode == LspMode::On {
                result.warnings.push(format!("{label} is not available"));
            }
            continue;
        };
        let timeout_ms = if options.lsp_timeout_ms == 0 {
            5000
        } else {
            options.lsp_timeout_ms.max(250)
        };
        let mut client = match Client::start(&command, root, timeout_ms) {
            Ok(client) => client,
            Err(error) => {
                result.warnings.push(format!("{label}: {error}"));
                continue;
            }
        };
        if client.supports("definitionProvider") {
            result.capabilities.push(format!("{label}:definition"));
        }
        if client.supports("callHierarchyProvider") {
            result.capabilities.push(format!("{label}:call_hierarchy"));
        }
        if !client.supports("definitionProvider") && !client.supports("callHierarchyProvider") {
            result.warnings.push(format!(
                "{label} advertised no supported semantic capabilities"
            ));
        }
        let language_files: Vec<(u32, &FileRecord)> = files
            .iter()
            .enumerate()
            .filter_map(|(id, file)| (file.language == language).then_some((id as u32, file)))
            .collect();
        for (_, file) in &language_files {
            let path = root.join(&file.path);
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let _ = client.notify(
                "textDocument/didOpen",
                json!({"textDocument": {"uri": path_to_uri(&path), "languageId": language.as_str(), "version": 1, "text": text}}),
            );
        }
        if client.supports("definitionProvider") {
            for call in calls.iter_mut() {
                let Some(caller) = symbols.get(call.caller as usize) else {
                    continue;
                };
                let file = &files[caller.file as usize];
                if file.language != language {
                    continue;
                }
                let uri = path_to_uri(&root.join(&file.path));
                let Ok(response) = client.request(
                "textDocument/definition",
                json!({"textDocument": {"uri": uri}, "position": {"line": call.line.saturating_sub(1), "character": utf16_column(root, file, call.line.saturating_sub(1), call.column)}}),
            ) else {
                continue;
            };
                let Some((target_uri, target_line)) = first_location(&response) else {
                    continue;
                };
                let Some(target_path) = uri_to_path(&target_uri) else {
                    continue;
                };
                let relative = target_path
                    .strip_prefix(root)
                    .unwrap_or(&target_path)
                    .to_string_lossy()
                    .replace('\\', "/");
                if let Some(target) = find_symbol(files, symbols, &relative, target_line) {
                    call.target = Some(target.id);
                    call.semantic = true;
                    result.resolved += 1;
                }
            }
        }

        if client.supports("callHierarchyProvider") {
            let mut seen_callers = HashSet::new();
            let callers: Vec<u32> = calls
                .iter()
                .map(|call| call.caller)
                .filter(|caller| {
                    symbols
                        .get(*caller as usize)
                        .is_some_and(|symbol| files[symbol.file as usize].language == language)
                })
                .filter(|caller| seen_callers.insert(*caller))
                .collect();
            for caller_id in callers.into_iter().take(2000) {
                let Some(caller) = symbols.get(caller_id as usize) else {
                    continue;
                };
                let file = &files[caller.file as usize];
                let uri = path_to_uri(&root.join(&file.path));
                let position = source_position(root, file, caller);
                let Ok(prepared) = client.request(
                    "textDocument/prepareCallHierarchy",
                    json!({"textDocument": {"uri": uri}, "position": position}),
                ) else {
                    continue;
                };
                let Some(item) = prepared.as_array().and_then(|items| items.first()) else {
                    continue;
                };
                let Ok(outgoing) =
                    client.request("callHierarchy/outgoingCalls", json!({"item": item}))
                else {
                    continue;
                };
                for edge in outgoing.as_array().into_iter().flatten() {
                    let Some(target) = edge.get("to") else {
                        continue;
                    };
                    let Some((target_uri, target_line)) = location_from_value(target) else {
                        continue;
                    };
                    let Some(target_path) = uri_to_path(&target_uri) else {
                        continue;
                    };
                    let relative = target_path
                        .strip_prefix(root)
                        .unwrap_or(&target_path)
                        .to_string_lossy()
                        .replace('\\', "/");
                    let Some(target_symbol) = find_symbol(files, symbols, &relative, target_line)
                    else {
                        continue;
                    };
                    let line = edge
                        .get("fromRanges")
                        .and_then(Value::as_array)
                        .and_then(|ranges| ranges.first())
                        .and_then(|range| range.get("start"))
                        .and_then(|start| start.get("line"))
                        .and_then(Value::as_u64)
                        .unwrap_or(caller.start_line.saturating_sub(1) as u64)
                        as u32
                        + 1;
                    if let Some(existing) = calls.iter_mut().find(|call| {
                        call.caller == caller_id
                            && call.target == Some(target_symbol.id)
                            && !call.synthetic
                    }) {
                        existing.semantic = true;
                        result.call_hierarchy += 1;
                    } else {
                        calls.push(Call {
                            caller: caller_id,
                            target: Some(target_symbol.id),
                            name: target_symbol.name.clone(),
                            line,
                            column: 0,
                            receiver_is_instance: false,
                            semantic: true,
                            synthetic: true,
                        });
                        result.call_hierarchy += 1;
                    }
                }
            }
        }
        result.servers.push(label.to_owned());
    }
    result
}

fn effective_command(
    explicit: Option<&str>,
    env_name: &str,
    fallback: &str,
    mode: LspMode,
) -> Option<String> {
    if mode == LspMode::Off {
        return None;
    }
    if let Some(command) = explicit {
        return Some(command.to_owned());
    }
    if let Ok(command) = std::env::var(env_name) {
        if !command.trim().is_empty() {
            return Some(command);
        }
    }
    let executable = fallback.split_whitespace().next().unwrap_or(fallback);
    command_exists(executable).then(|| fallback.to_owned())
}

fn command_exists(executable: &str) -> bool {
    std::process::Command::new("sh")
        .arg("-c")
        .arg(format!(
            "command -v {} >/dev/null 2>&1",
            shell_quote(executable)
        ))
        .status()
        .is_ok_and(|status| status.success())
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn find_symbol<'a>(
    files: &[FileRecord],
    symbols: &'a [Symbol],
    relative: &str,
    line: u32,
) -> Option<&'a Symbol> {
    symbols.iter().find(|symbol| {
        files[symbol.file as usize].path == relative
            && symbol.start_line <= line + 1
            && symbol.end_line > line
    })
}

fn source_position(root: &Path, file: &FileRecord, symbol: &Symbol) -> Value {
    let line = symbol.start_line.saturating_sub(1);
    let byte_column = std::fs::read_to_string(root.join(&file.path))
        .ok()
        .and_then(|source| {
            source
                .lines()
                .nth(line as usize)
                .and_then(|text| text.find(&symbol.name))
        })
        .map(|column| column as u32)
        .unwrap_or(0);
    let character = utf16_column(root, file, line, byte_column);
    json!({"line": line, "character": character})
}

fn utf16_column(root: &Path, file: &FileRecord, line: u32, byte_column: u32) -> u32 {
    let Ok(source) = std::fs::read_to_string(root.join(&file.path)) else {
        return byte_column;
    };
    let Some(text) = source.lines().nth(line as usize) else {
        return byte_column;
    };
    text.as_bytes()
        .get(..byte_column as usize)
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .map(|prefix| prefix.encode_utf16().count() as u32)
        .unwrap_or(byte_column)
}

struct Client {
    child: Child,
    stdin: ChildStdin,
    messages: Receiver<std::result::Result<Value, String>>,
    timeout: Duration,
    next_id: u64,
    capabilities: Value,
}

impl Client {
    fn start(command: &str, root: &Path, timeout_ms: u64) -> Result<Self> {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("failed to start LSP command: {command}"))?;
        if let Some(status) = child.try_wait()? {
            return Err(anyhow::anyhow!(
                "LSP command exited before initialization: {status}"
            ));
        }
        let stdin = child.stdin.take().context("LSP stdin unavailable")?;
        let stdout = child.stdout.take().context("LSP stdout unavailable")?;
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || read_messages(stdout, sender));
        let mut client = Self {
            child,
            stdin,
            messages: receiver,
            timeout: Duration::from_millis(timeout_ms),
            next_id: 1,
            capabilities: json!({}),
        };
        let initialize = client.request(
            "initialize",
            json!({"processId": std::process::id(), "rootUri": path_to_uri(root), "workspaceFolders": [{"uri": path_to_uri(root), "name": root.file_name().and_then(|v| v.to_str()).unwrap_or("workspace")}], "capabilities": {}}),
        )?;
        client.capabilities = initialize
            .get("capabilities")
            .cloned()
            .unwrap_or_else(|| json!({}));
        client.notify("initialized", json!({}))?;
        Ok(client)
    }

    fn supports(&self, capability: &str) -> bool {
        self.capabilities
            .get(capability)
            .is_some_and(|value| value.as_bool().unwrap_or(true))
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        self.send(json!({"jsonrpc":"2.0","method":method,"params":params}))
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))?;
        loop {
            let message = self
                .messages
                .recv_timeout(self.timeout)
                .map_err(|error| match error {
                    mpsc::RecvTimeoutError::Timeout => {
                        anyhow::anyhow!("LSP response timeout for {method}")
                    }
                    mpsc::RecvTimeoutError::Disconnected => {
                        anyhow::anyhow!("LSP process closed its output for {method}")
                    }
                })?
                .map_err(|error| anyhow::anyhow!(error))?;
            if message.get("id").and_then(Value::as_u64) == Some(id) {
                if let Some(error) = message.get("error") {
                    return Err(anyhow::anyhow!(error.to_string()));
                }
                return Ok(message.get("result").cloned().unwrap_or(Value::Null));
            }
        }
    }

    fn send(&mut self, message: Value) -> Result<()> {
        let body = serde_json::to_vec(&message)?;
        write!(self.stdin, "Content-Length: {}\r\n\r\n", body.len())?;
        self.stdin.write_all(&body)?;
        self.stdin.flush()?;
        Ok(())
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn read_messages<R: Read>(reader: R, sender: mpsc::Sender<std::result::Result<Value, String>>) {
    let mut reader = BufReader::new(reader);
    loop {
        let mut length = None;
        loop {
            let mut line = String::new();
            if reader
                .read_line(&mut line)
                .ok()
                .filter(|size| *size > 0)
                .is_none()
            {
                return;
            }
            if line == "\r\n" || line == "\n" {
                break;
            }
            if let Some(value) = line.strip_prefix("Content-Length:") {
                length = value.trim().parse::<usize>().ok();
            }
        }
        let Some(length) = length else { continue };
        let mut body = vec![0; length];
        if reader.read_exact(&mut body).is_err() {
            return;
        }
        let parsed = serde_json::from_slice(&body).map_err(|error| error.to_string());
        if sender.send(parsed).is_err() {
            return;
        }
    }
}

fn first_location(value: &Value) -> Option<(String, u32)> {
    let location = value
        .as_array()
        .and_then(|items| items.first())
        .unwrap_or(value);
    location_from_value(location)
}

fn location_from_value(location: &Value) -> Option<(String, u32)> {
    let uri = location
        .get("uri")
        .or_else(|| location.get("targetUri"))?
        .as_str()?
        .to_owned();
    let range = location
        .get("range")
        .or_else(|| location.get("targetRange"))?;
    let line = range.get("start")?.get("line")?.as_u64()? as u32;
    Some((uri, line))
}

fn path_to_uri(path: &Path) -> String {
    format!("file://{}", path.to_string_lossy().replace(' ', "%20"))
}

fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let path = uri.strip_prefix("file://")?.replace("%20", " ");
    Some(PathBuf::from(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_definition_location() {
        let value =
            json!([{"uri":"file:///tmp/Foo.java","range":{"start":{"line":7,"character":2}}}]);
        assert_eq!(
            first_location(&value),
            Some(("file:///tmp/Foo.java".into(), 7))
        );
    }

    #[test]
    fn round_trips_basic_file_uri() {
        let path = Path::new("/tmp/a file.java");
        assert_eq!(uri_to_path(&path_to_uri(path)), Some(path.to_path_buf()));
    }
}
