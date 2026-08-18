use super::{compact_signature, text};
use crate::model::{ExtractedFile, Language, LocalCall, LocalSymbol};
use anyhow::{Context, Result};
use std::path::Path;
use tree_sitter::{Language as TsLanguage, Node, Parser};

pub fn extract_javascript(path: String, source: &str) -> Result<ExtractedFile> {
    extract(
        path,
        source,
        Language::JavaScript,
        tree_sitter_javascript::LANGUAGE.into(),
    )
}

pub fn extract_typescript(path: String, source: &str, file: &Path) -> Result<ExtractedFile> {
    let tsx = matches!(
        file.extension().and_then(|value| value.to_str()),
        Some("tsx")
    );
    let grammar = if tsx {
        tree_sitter_typescript::LANGUAGE_TSX.into()
    } else {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
    };
    extract(path, source, Language::TypeScript, grammar)
}

pub fn extract_rust(path: String, source: &str) -> Result<ExtractedFile> {
    extract(
        path,
        source,
        Language::Rust,
        tree_sitter_rust::LANGUAGE.into(),
    )
}

pub fn extract_dart(path: String, source: &str) -> Result<ExtractedFile> {
    extract(
        path,
        source,
        Language::Dart,
        tree_sitter_dart::LANGUAGE.into(),
    )
}

fn extract(
    path: String,
    source: &str,
    language: Language,
    grammar: TsLanguage,
) -> Result<ExtractedFile> {
    let mut parser = Parser::new();
    parser
        .set_language(&grammar)
        .context("failed to initialize tree-sitter grammar")?;
    let tree = parser
        .parse(source, None)
        .context("failed to parse source")?;
    let mut visitor = Visitor {
        source,
        language,
        symbols: Vec::new(),
        calls: Vec::new(),
        scopes: Vec::new(),
    };
    visitor.visit(tree.root_node());
    Ok(ExtractedFile {
        path,
        language,
        bytes: source.len() as u64,
        modified_ns: 0,
        symbols: visitor.symbols,
        calls: visitor.calls,
    })
}

struct Visitor<'a> {
    source: &'a str,
    language: Language,
    symbols: Vec<LocalSymbol>,
    calls: Vec<LocalCall>,
    scopes: Vec<usize>,
}

impl Visitor<'_> {
    fn visit(&mut self, node: Node<'_>) {
        let symbol = self.symbol(node);
        if let Some((name, kind)) = symbol {
            let parent = self.scopes.last().copied();
            let qualified_name = parent
                .and_then(|id| {
                    self.symbols
                        .get(id)
                        .map(|item| format!("{}::{name}", item.qualified_name))
                })
                .unwrap_or_else(|| name.clone());
            let id = self.symbols.len();
            self.symbols.push(LocalSymbol {
                name,
                qualified_name,
                kind,
                start_line: node.start_position().row as u32 + 1,
                end_line: node.end_position().row as u32 + 1,
                signature: compact_signature(text(node, self.source)),
                parent,
            });
            self.scopes.push(id);
            self.visit_children(node);
            self.scopes.pop();
            return;
        }
        if let Some(name) = self.call_name(node) {
            if let Some(&caller) = self.scopes.last() {
                self.calls.push(LocalCall {
                    caller,
                    receiver_is_instance: self.instance_call(node),
                    name,
                    line: node.start_position().row as u32 + 1,
                    column: node.start_position().column as u32,
                });
            }
        }
        self.visit_children(node);
    }

    fn visit_children(&mut self, node: Node<'_>) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.visit(child);
        }
    }

    fn symbol(&self, node: Node<'_>) -> Option<(String, String)> {
        let kind = node.kind();
        let mapped = match self.language {
            Language::JavaScript | Language::TypeScript => match kind {
                "function_declaration"
                | "generator_function_declaration"
                | "arrow_function"
                | "function_expression" => "function",
                "class_declaration" | "abstract_class_declaration" => "class",
                "method_definition" | "public_field_definition" => "method",
                "interface_declaration" => "interface",
                "type_alias_declaration" => "type",
                "enum_declaration" => "enum",
                "variable_declarator"
                    if node.child_by_field_name("value").is_some_and(|value| {
                        matches!(value.kind(), "arrow_function" | "function_expression")
                    }) =>
                {
                    "function"
                }
                _ => return None,
            },
            Language::Rust => match kind {
                "function_item" => "function",
                "struct_item" => "struct",
                "enum_item" => "enum",
                "trait_item" => "trait",
                "mod_item" => "module",
                "impl_item" => "impl",
                "const_item" | "static_item" => "constant",
                _ => return None,
            },
            Language::Dart => match kind {
                "class_declaration" | "extension_type_declaration" => "class",
                "enum_declaration" => "enum",
                "mixin_declaration" => "mixin",
                "extension_declaration" => "extension",
                "function_signature" | "method_declaration" | "constructor_signature" => "function",
                _ => return None,
            },
            _ => return None,
        };
        let name = node
            .child_by_field_name("name")
            .or_else(|| node.child_by_field_name("declarator"))
            .or_else(|| first_identifier(node))
            .map(|name| text(name, self.source).trim_matches('"').to_owned())?;
        (!name.is_empty()).then_some((name, mapped.to_owned()))
    }

    fn call_name(&self, node: Node<'_>) -> Option<String> {
        let is_call = matches!(
            node.kind(),
            "call_expression"
                | "new_expression"
                | "method_call_expression"
                | "macro_invocation"
                | "method_invocation"
                | "instance_creation_expression"
                | "function_expression_invocation"
        );
        if !is_call {
            return None;
        }
        let target = node
            .child_by_field_name("function")
            .or_else(|| node.child_by_field_name("method"))
            .or_else(|| node.child_by_field_name("name"))
            .or_else(|| node.child_by_field_name("macro"))
            .or_else(|| first_identifier(node))?;
        let value = text(target, self.source).trim();
        let value = value
            .rsplit(['.', ':'])
            .next()
            .unwrap_or(value)
            .trim_end_matches('!');
        (!value.is_empty()).then_some(value.to_owned())
    }

    fn instance_call(&self, node: Node<'_>) -> bool {
        matches!(node.kind(), "method_call_expression" | "method_invocation")
            || node.child_by_field_name("object").is_some()
    }
}

fn first_identifier(node: Node<'_>) -> Option<Node<'_>> {
    let mut cursor = node.walk();
    let found = node.named_children(&mut cursor).find(|child| {
        matches!(
            child.kind(),
            "identifier" | "type_identifier" | "property_identifier"
        )
    });
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(file: ExtractedFile) -> Vec<String> {
        file.symbols.into_iter().map(|symbol| symbol.name).collect()
    }

    #[test]
    fn extracts_javascript_and_typescript_symbols_and_calls() {
        let javascript = extract_javascript(
            "app.js".into(),
            "class App { run() { return helper(); } } function helper() {}",
        )
        .unwrap();
        assert!(names(javascript).contains(&"App".to_owned()));
        let typescript = extract_typescript(
            "app.ts".into(),
            "interface Config {} const boot = () => helper(); function helper() {}",
            Path::new("app.ts"),
        )
        .unwrap();
        assert!(names(typescript).contains(&"boot".to_owned()));
    }

    #[test]
    fn extracts_rust_symbols_and_calls() {
        let file = extract_rust(
            "lib.rs".into(),
            "struct App; impl App { fn run(&self) { helper(); } } fn helper() {}",
        )
        .unwrap();
        assert!(names(file).contains(&"App".to_owned()));
    }

    #[test]
    fn extracts_dart_symbols_and_calls() {
        let file = extract_dart(
            "main.dart".into(),
            "class App { void run() { helper(); } } void helper() {}",
        )
        .unwrap();
        assert!(names(file).contains(&"App".to_owned()));
    }
}
