use crate::model::{Index, Symbol};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};

pub fn search(index: &Index, query: &str, kind: Option<&str>, limit: usize) -> Value {
    let needle = query.to_lowercase();
    let mut matches: Vec<(&Symbol, u8)> = index
        .symbols
        .iter()
        .filter(|symbol| kind.is_none_or(|kind| symbol.kind == kind))
        .filter_map(|symbol| {
            let name = symbol.name.to_lowercase();
            let qualified = symbol.qualified_name.to_lowercase();
            let qualified_query = needle.contains('.') || needle.contains('/');
            let score = if name == needle {
                0
            } else if name.starts_with(&needle) {
                1
            } else if name.contains(&needle) {
                2
            } else if qualified_query && qualified.contains(&needle) {
                3
            } else {
                return None;
            };
            Some((symbol, score))
        })
        .collect();
    matches.sort_by(|(a, sa), (b, sb)| {
        sa.cmp(sb)
            .then_with(|| a.qualified_name.len().cmp(&b.qualified_name.len()))
    });
    let total = matches.len();
    let results: Vec<Value> = matches
        .into_iter()
        .take(limit)
        .map(|(symbol, _)| symbol_row(index, symbol))
        .collect();
    json!({"total": total, "results": results})
}

pub fn symbol(index: &Index, qualified_name: &str, context: usize) -> Result<Value> {
    let symbol = resolve_symbol(index, qualified_name)?;
    let file = &index.files[symbol.file as usize];
    let path = std::path::Path::new(&index.root).join(&file.path);
    let source = std::fs::read_to_string(&path)?;
    let lines: Vec<&str> = source.lines().collect();
    let start = symbol.start_line.saturating_sub(1 + context as u32) as usize;
    let end = (symbol.end_line as usize + context).min(lines.len());
    let snippet = lines[start..end].join("\n");
    Ok(json!({
        "symbol": symbol_row(index, symbol),
        "range": [start + 1, end],
        "code": snippet
    }))
}

pub fn trace(
    index: &Index,
    name: &str,
    direction: &str,
    depth: usize,
    limit: usize,
) -> Result<Value> {
    let start = resolve_symbol(index, name)?.id;
    let mut adjacency: HashMap<u32, Vec<(u32, u32)>> = HashMap::new();
    for call in &index.calls {
        if let Some(target) = call.target {
            let (from, to) = if direction == "inbound" {
                (target, call.caller)
            } else {
                (call.caller, target)
            };
            adjacency.entry(from).or_default().push((to, call.line));
        }
    }
    let mut queue = VecDeque::from([(start, 0usize)]);
    let mut seen = HashSet::from([start]);
    let mut paths = Vec::new();
    while let Some((from, distance)) = queue.pop_front() {
        if distance >= depth || paths.len() >= limit {
            continue;
        }
        for &(to, line) in adjacency.get(&from).into_iter().flatten() {
            if paths.len() >= limit {
                break;
            }
            paths.push(json!({"from": symbol_selector(&index.symbols[from as usize]), "to": symbol_selector(&index.symbols[to as usize]), "line": line, "depth": distance + 1}));
            if seen.insert(to) {
                queue.push_back((to, distance + 1));
            }
        }
    }
    Ok(
        json!({"start": symbol_selector(&index.symbols[start as usize]), "direction": direction, "paths": paths, "truncated": paths.len() >= limit}),
    )
}

pub fn overview(index: &Index) -> Value {
    let mut languages: HashMap<&str, usize> = HashMap::new();
    let mut kinds: HashMap<&str, usize> = HashMap::new();
    for file in &index.files {
        *languages.entry(file.language.as_str()).or_default() += 1;
    }
    for symbol in &index.symbols {
        *kinds.entry(&symbol.kind).or_default() += 1;
    }
    let resolved = index
        .calls
        .iter()
        .filter(|call| call.target.is_some())
        .count();
    json!({
        "root": index.root,
        "files": index.files.len(),
        "symbols": index.symbols.len(),
        "calls": {"total": index.calls.len(), "resolved": resolved},
        "languages": languages,
        "kinds": kinds,
        "index_ms": index.elapsed_ms,
        "incremental": {"parsed": index.parsed_files, "reused": index.reused_files},
        "lsp": {
            "mode": index.lsp_mode,
            "servers": index.lsp_servers,
            "capabilities": index.lsp_capabilities,
            "resolved": index.lsp_resolved,
            "call_hierarchy": index.lsp_call_hierarchy,
            "warnings": index.lsp_warnings
        }
    })
}

fn resolve_symbol<'a>(index: &'a Index, value: &str) -> Result<&'a Symbol> {
    // Search results use this line-qualified selector for overloaded Java
    // methods. It is stable for a pinned source revision and lets an MCP
    // client trace the exact overload it discovered rather than whichever
    // same-named method happened to be indexed first.
    if let Some((qualified_name, line)) = value.rsplit_once('@') {
        if let Ok(line) = line.parse::<u32>() {
            if let Some(symbol) = index
                .symbols
                .iter()
                .find(|symbol| symbol.qualified_name == qualified_name && symbol.start_line == line)
            {
                return Ok(symbol);
            }
        }
    }
    if let Some(symbol) = index
        .symbols
        .iter()
        .find(|symbol| symbol.qualified_name == value)
    {
        return Ok(symbol);
    }
    let matches: Vec<&Symbol> = index
        .symbols
        .iter()
        .filter(|symbol| symbol.name == value)
        .collect();
    match matches.as_slice() {
        [symbol] => Ok(*symbol),
        [] => Err(anyhow!("symbol not found: {value}")),
        _ => Err(anyhow!("ambiguous symbol {value}; use a qualified name")),
    }
}

fn symbol_row(index: &Index, symbol: &Symbol) -> Value {
    json!({
        "name": symbol_selector(symbol),
        "display_name": symbol.qualified_name,
        "kind": symbol.kind,
        "at": format!("{}:{}", index.files[symbol.file as usize].path, symbol.start_line),
        "sig": symbol.signature
    })
}

fn symbol_selector(symbol: &Symbol) -> String {
    format!("{}@{}", symbol.qualified_name, symbol.start_line)
}
