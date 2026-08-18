use crate::languages;
use crate::lsp;
use crate::model::{
    Call, ExtractedFile, FileRecord, Index, LocalCall, LocalSymbol, Symbol, INDEX_VERSION,
};
use anyhow::{anyhow, Context, Result};
use ignore::WalkBuilder;
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Debug, Clone, Default)]
pub struct BuildOptions {
    pub jdtls_command: Option<String>,
    pub clojure_lsp_command: Option<String>,
    pub typescript_lsp_command: Option<String>,
    pub rust_analyzer_command: Option<String>,
    pub dart_lsp_command: Option<String>,
    pub lsp_timeout_ms: u64,
    pub lsp_mode: lsp::LspMode,
}

pub fn build_with_options(root: &Path, options: &BuildOptions) -> Result<Index> {
    let started = Instant::now();
    let root = root
        .canonicalize()
        .with_context(|| format!("invalid repository path: {}", root.display()))?;
    let paths: Vec<(PathBuf, crate::model::Language)> = WalkBuilder::new(&root)
        .hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .filter_entry(|entry| {
            entry.file_name() != ".git"
                && entry.file_name() != "target"
                && entry.file_name() != ".connectome"
        })
        .build()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_some_and(|kind| kind.is_file()))
        .filter_map(|entry| {
            languages::detect(entry.path()).map(|language| (entry.into_path(), language))
        })
        .collect();

    let previous = crate::store::load(&crate::store::default_path(&root)).ok();
    let previous_files: HashMap<&str, u32> = previous
        .as_ref()
        .map(|index| {
            index
                .files
                .iter()
                .enumerate()
                .map(|(id, file)| (file.path.as_str(), id as u32))
                .collect()
        })
        .unwrap_or_default();
    let mut reused_files = 0u32;
    let mut extracted = paths
        .par_iter()
        .map(|(path, language)| {
            let relative = path
                .strip_prefix(&root)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");
            if let (Some(old), Some(file_id)) =
                (previous.as_ref(), previous_files.get(relative.as_str()))
            {
                let metadata = std::fs::metadata(path)?;
                let modified_ns = metadata
                    .modified()?
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos();
                let file = &old.files[*file_id as usize];
                if file.bytes == metadata.len()
                    && file.modified_ns == modified_ns
                    && file.language == *language
                {
                    return Ok(reuse_file(old, *file_id));
                }
            }
            languages::extract(&root, path, *language)
        })
        .collect::<Result<Vec<_>>>()?;
    extracted.sort_by(|a, b| a.path.cmp(&b.path));

    if let Some(old) = previous.as_ref() {
        reused_files = extracted
            .iter()
            .filter(|file| {
                previous_files.get(file.path.as_str()).is_some_and(|id| {
                    let prior = &old.files[*id as usize];
                    prior.bytes == file.bytes && prior.modified_ns == file.modified_ns
                })
            })
            .count() as u32;
    }

    let mut files = Vec::with_capacity(extracted.len());
    let mut symbols = Vec::new();
    let mut pending_calls = Vec::new();
    for file in extracted {
        let file_id = files.len() as u32;
        files.push(FileRecord {
            path: file.path,
            language: file.language,
            bytes: file.bytes,
            modified_ns: file.modified_ns,
        });
        let symbol_base = symbols.len() as u32;
        for (local_id, symbol) in file.symbols.into_iter().enumerate() {
            symbols.push(Symbol {
                id: symbol_base + local_id as u32,
                name: symbol.name,
                qualified_name: symbol.qualified_name,
                kind: symbol.kind,
                file: file_id,
                start_line: symbol.start_line,
                end_line: symbol.end_line,
                signature: symbol.signature,
                parent: symbol.parent.map(|id| symbol_base + id as u32),
            });
        }
        pending_calls.extend(file.calls.into_iter().map(|call| {
            (
                symbol_base + call.caller as u32,
                call.name,
                call.line,
                call.column,
                call.receiver_is_instance,
            )
        }));
    }

    let mut by_name: HashMap<&str, Vec<u32>> = HashMap::new();
    for symbol in &symbols {
        by_name.entry(&symbol.name).or_default().push(symbol.id);
    }
    let mut calls: Vec<Call> = pending_calls
        .into_iter()
        .map(|(caller, name, line, column, prefer_instance)| {
            let candidates = by_name.get(name.as_str());
            let target =
                candidates.and_then(|ids| choose_target(&symbols, caller, ids, prefer_instance));
            Call {
                caller,
                target,
                name,
                line,
                column,
                receiver_is_instance: prefer_instance,
                semantic: false,
                synthetic: false,
            }
        })
        .collect();

    // Clojure calls to a defmulti dynamically dispatch to one of its
    // defmethod bodies. Add explicit, source-backed edges so trace_calls can
    // surface the implementation without making the model retrieve files.
    // Method qualified names end in "::<dispatch>" (see clojure extractor).
    let multimethods: Vec<(u32, String)> = symbols
        .iter()
        .filter(|symbol| symbol.kind == "multimethod")
        .map(|symbol| (symbol.id, symbol.qualified_name.clone()))
        .collect();
    for (multimethod_id, qualified_name) in multimethods {
        let prefix = format!("{qualified_name}::");
        for method in symbols
            .iter()
            .filter(|symbol| symbol.kind == "method" && symbol.qualified_name.starts_with(&prefix))
        {
            calls.push(Call {
                caller: multimethod_id,
                target: Some(method.id),
                name: method.name.clone(),
                line: method.start_line,
                column: 0,
                receiver_is_instance: false,
                semantic: false,
                synthetic: true,
            });
        }
    }

    let lsp_result = lsp::resolve_calls(&root, &files, &symbols, &mut calls, options);

    if files.is_empty() {
        return Err(anyhow!(
            "no supported source files found under {}",
            root.display()
        ));
    }
    let parsed_files = files.len() as u32 - reused_files;
    Ok(Index {
        version: INDEX_VERSION,
        root: root.to_string_lossy().into_owned(),
        files,
        symbols,
        calls,
        elapsed_ms: started.elapsed().as_millis() as u64,
        parsed_files,
        reused_files,
        lsp_resolved: lsp_result.resolved,
        lsp_call_hierarchy: lsp_result.call_hierarchy,
        lsp_servers: lsp_result.servers,
        lsp_capabilities: lsp_result.capabilities,
        lsp_warnings: lsp_result.warnings,
        lsp_mode: options.lsp_mode.as_str().to_owned(),
    })
}

fn reuse_file(index: &Index, file_id: u32) -> ExtractedFile {
    let file = &index.files[file_id as usize];
    let selected: Vec<&Symbol> = index
        .symbols
        .iter()
        .filter(|symbol| symbol.file == file_id)
        .collect();
    let local_ids: HashMap<u32, usize> = selected
        .iter()
        .enumerate()
        .map(|(local, symbol)| (symbol.id, local))
        .collect();
    let symbols = selected
        .iter()
        .map(|symbol| LocalSymbol {
            name: symbol.name.clone(),
            qualified_name: symbol.qualified_name.clone(),
            kind: symbol.kind.clone(),
            start_line: symbol.start_line,
            end_line: symbol.end_line,
            signature: symbol.signature.clone(),
            parent: symbol.parent.and_then(|id| local_ids.get(&id).copied()),
        })
        .collect();
    let calls = index
        .calls
        .iter()
        .filter(|call| !call.synthetic)
        .filter_map(|call| {
            local_ids.get(&call.caller).map(|caller| LocalCall {
                caller: *caller,
                name: call.name.clone(),
                line: call.line,
                column: call.column,
                receiver_is_instance: call.receiver_is_instance,
            })
        })
        .collect();
    ExtractedFile {
        path: file.path.clone(),
        language: file.language,
        bytes: file.bytes,
        modified_ns: file.modified_ns,
        symbols,
        calls,
    }
}

fn choose_target(
    symbols: &[Symbol],
    caller: u32,
    candidates: &[u32],
    prefer_instance: bool,
) -> Option<u32> {
    if candidates.len() == 1 {
        return candidates.first().copied();
    }
    let caller = symbols.get(caller as usize)?;
    candidates
        .iter()
        .copied()
        .find(|id| symbols[*id as usize].kind == "multimethod")
        .or_else(|| {
            let caller_is_static = caller.signature.contains(" static ");
            candidates.iter().copied().find(|id| {
                let target = &symbols[*id as usize];
                *id != caller.id
                    && target.file == caller.file
                    && target.kind == "method"
                    && target.signature.contains(" static ")
                        == if prefer_instance {
                            false
                        } else {
                            caller_is_static
                        }
            })
        })
        .or_else(|| {
            candidates
                .iter()
                .copied()
                .find(|id| symbols[*id as usize].file == caller.file)
        })
}
