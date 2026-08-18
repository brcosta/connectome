mod clojure;
mod java;

use crate::model::{ExtractedFile, Language};
use anyhow::{Context, Result};
use std::path::Path;

pub fn detect(path: &Path) -> Option<Language> {
    match path.extension()?.to_str()? {
        "java" => Some(Language::Java),
        "clj" | "cljs" | "cljc" | "edn" => Some(Language::Clojure),
        _ => None,
    }
}

pub fn extract(root: &Path, path: &Path, language: Language) -> Result<ExtractedFile> {
    let source = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let relative = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    let metadata = std::fs::metadata(path)?;
    let modified_ns = metadata
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut extracted = match language {
        Language::Java => java::extract(relative, &source)?,
        Language::Clojure => clojure::extract(relative, &source)?,
    };
    extracted.modified_ns = modified_ns;
    Ok(extracted)
}

pub(crate) fn text<'a>(node: tree_sitter::Node<'_>, source: &'a str) -> &'a str {
    source.get(node.byte_range()).unwrap_or("")
}

pub(crate) fn compact_signature(value: &str) -> String {
    let head = value.split('{').next().unwrap_or(value);
    let compact = head.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.len() > 240 {
        format!("{}…", &compact[..240])
    } else {
        compact
    }
}
