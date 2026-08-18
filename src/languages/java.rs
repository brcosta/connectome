use super::{compact_signature, text};
use crate::model::{ExtractedFile, Language, LocalCall, LocalSymbol};
use anyhow::{anyhow, Result};
use tree_sitter::{Node, Parser};

pub fn extract(path: String, source: &str) -> Result<ExtractedFile> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_java::LANGUAGE.into())
        .map_err(|e| anyhow!("cannot load Java parser: {e}"))?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| anyhow!("Java parse cancelled"))?;
    let package = find_package(tree.root_node(), source);
    let mut state = State {
        source,
        package,
        symbols: Vec::new(),
        calls: Vec::new(),
    };
    state.walk(tree.root_node(), None, &[]);
    Ok(ExtractedFile {
        path,
        language: Language::Java,
        bytes: source.len() as u64,
        modified_ns: 0,
        symbols: state.symbols,
        calls: state.calls,
    })
}

fn find_package(root: Node<'_>, source: &str) -> String {
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        if child.kind() == "package_declaration" {
            return text(child, source)
                .trim()
                .trim_start_matches("package")
                .trim()
                .trim_end_matches(';')
                .to_owned();
        }
    }
    String::new()
}

struct State<'a> {
    source: &'a str,
    package: String,
    symbols: Vec<LocalSymbol>,
    calls: Vec<LocalCall>,
}

impl State<'_> {
    fn walk(&mut self, node: Node<'_>, current_callable: Option<usize>, types: &[String]) {
        let definition = match node.kind() {
            "class_declaration" => Some("class"),
            "interface_declaration" => Some("interface"),
            "enum_declaration" => Some("enum"),
            "record_declaration" => Some("record"),
            "annotation_type_declaration" => Some("annotation"),
            "method_declaration" => Some("method"),
            "constructor_declaration" => Some("constructor"),
            _ => None,
        };

        let mut next_callable = current_callable;
        let mut next_types = types.to_vec();
        if let Some(kind) = definition {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, self.source).to_owned();
                let mut parts = Vec::new();
                if !self.package.is_empty() {
                    parts.push(self.package.clone());
                }
                parts.extend(types.iter().cloned());
                parts.push(name.clone());
                let parent = nearest_type(&self.symbols, types);
                let index = self.symbols.len();
                self.symbols.push(LocalSymbol {
                    name: name.clone(),
                    qualified_name: parts.join("."),
                    kind: kind.to_owned(),
                    start_line: node.start_position().row as u32 + 1,
                    end_line: node.end_position().row as u32 + 1,
                    signature: compact_signature(text(node, self.source)),
                    parent,
                });
                if matches!(kind, "method" | "constructor") {
                    next_callable = Some(index);
                } else {
                    next_types.push(name);
                }
            }
        }

        if node.kind() == "method_invocation" {
            if let (Some(caller), Some(name_node)) =
                (current_callable, node.child_by_field_name("name"))
            {
                let name = text(name_node, self.source);
                // Preserve the one receiver fact that tree-sitter can state
                // without type checking: `new Type(...).method()` is an
                // instance call. The indexer uses this only to choose between
                // same-named static and instance overloads.
                let receiver_is_instance = node
                    .child_by_field_name("object")
                    .is_some_and(|object| object.kind() == "object_creation_expression");
                self.calls.push(LocalCall {
                    caller,
                    name: name.to_owned(),
                    line: node.start_position().row as u32 + 1,
                    column: node.start_position().column as u32,
                    receiver_is_instance,
                });
            }
        } else if node.kind() == "object_creation_expression" {
            if let (Some(caller), Some(type_node)) =
                (current_callable, node.child_by_field_name("type"))
            {
                self.calls.push(LocalCall {
                    caller,
                    name: text(type_node, self.source).to_owned(),
                    line: node.start_position().row as u32 + 1,
                    column: node.start_position().column as u32,
                    receiver_is_instance: false,
                });
            }
        }

        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.walk(child, next_callable, &next_types);
        }
    }
}

fn nearest_type(symbols: &[LocalSymbol], types: &[String]) -> Option<usize> {
    let name = types.last()?;
    symbols
        .iter()
        .enumerate()
        .rev()
        .find(|(_, symbol)| {
            &symbol.name == name
                && matches!(
                    symbol.kind.as_str(),
                    "class" | "interface" | "enum" | "record" | "annotation"
                )
        })
        .map(|(index, _)| index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_types_methods_and_calls() {
        let source = r#"
            package dev.connectome;
            class Greeter {
                String greet(String name) { return decorate(name); }
                String decorate(String name) { return "Hi " + name; }
            }
        "#;
        let file = extract("Greeter.java".into(), source).unwrap();
        assert_eq!(
            file.symbols
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>(),
            ["Greeter", "greet", "decorate"]
        );
        assert_eq!(
            file.symbols[1].qualified_name,
            "dev.connectome.Greeter.greet"
        );
        assert_eq!(file.calls[0].name, "decorate");
    }
}
