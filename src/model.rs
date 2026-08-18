use serde::{Deserialize, Serialize};

pub const INDEX_VERSION: u16 = 9;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Index {
    pub version: u16,
    pub root: String,
    pub files: Vec<FileRecord>,
    pub symbols: Vec<Symbol>,
    pub calls: Vec<Call>,
    pub elapsed_ms: u64,
    pub parsed_files: u32,
    pub reused_files: u32,
    pub lsp_resolved: u32,
    pub lsp_call_hierarchy: u32,
    pub lsp_servers: Vec<String>,
    pub lsp_capabilities: Vec<String>,
    pub lsp_warnings: Vec<String>,
    pub lsp_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRecord {
    pub path: String,
    pub language: Language,
    pub bytes: u64,
    pub modified_ns: u128,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    Java,
    Clojure,
    JavaScript,
    TypeScript,
    Rust,
    Dart,
}

impl Language {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Java => "java",
            Self::Clojure => "clojure",
            Self::JavaScript => "javascript",
            Self::TypeScript => "typescript",
            Self::Rust => "rust",
            Self::Dart => "dart",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    pub id: u32,
    pub name: String,
    pub qualified_name: String,
    pub kind: String,
    pub file: u32,
    pub start_line: u32,
    pub end_line: u32,
    pub signature: String,
    pub parent: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Call {
    pub caller: u32,
    pub target: Option<u32>,
    pub name: String,
    pub line: u32,
    pub column: u32,
    pub receiver_is_instance: bool,
    pub semantic: bool,
    pub synthetic: bool,
}

#[derive(Debug)]
pub struct ExtractedFile {
    pub path: String,
    pub language: Language,
    pub bytes: u64,
    pub modified_ns: u128,
    pub symbols: Vec<LocalSymbol>,
    pub calls: Vec<LocalCall>,
}

#[derive(Debug)]
pub struct LocalSymbol {
    pub name: String,
    pub qualified_name: String,
    pub kind: String,
    pub start_line: u32,
    pub end_line: u32,
    pub signature: String,
    pub parent: Option<usize>,
}

#[derive(Debug)]
pub struct LocalCall {
    pub caller: usize,
    pub name: String,
    pub line: u32,
    pub column: u32,
    pub receiver_is_instance: bool,
}
