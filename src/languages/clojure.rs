use super::{compact_signature, text};
use crate::model::{ExtractedFile, Language, LocalCall, LocalSymbol};
use anyhow::{anyhow, Result};
use tree_sitter::{Node, Parser};

pub fn extract(path: String, source: &str) -> Result<ExtractedFile> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_clojure::LANGUAGE.into())
        .map_err(|e| anyhow!("cannot load Clojure parser: {e}"))?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| anyhow!("Clojure parse cancelled"))?;
    let mut state = State {
        source,
        namespace: String::new(),
        symbols: Vec::new(),
        calls: Vec::new(),
    };
    state.walk(tree.root_node(), None);
    Ok(ExtractedFile {
        path,
        language: Language::Clojure,
        bytes: source.len() as u64,
        modified_ns: 0,
        symbols: state.symbols,
        calls: state.calls,
    })
}

struct State<'a> {
    source: &'a str,
    namespace: String,
    symbols: Vec<LocalSymbol>,
    calls: Vec<LocalCall>,
}

impl State<'_> {
    fn walk(&mut self, node: Node<'_>, current_callable: Option<usize>) {
        if node.kind() == "list_lit" {
            let children = named_children(node);
            let head = children
                .first()
                .map(|n| text(*n, self.source))
                .unwrap_or("");
            if head == "ns" {
                if let Some(name) = children.get(1) {
                    self.namespace = text(*name, self.source).to_owned();
                }
            }

            let kind = match head {
                "defn" | "defn-" => Some("function"),
                "defmacro" => Some("macro"),
                "def" | "defonce" => Some("var"),
                "defrecord" => Some("record"),
                "deftype" => Some("type"),
                "defprotocol" => Some("protocol"),
                "defmulti" => Some("multimethod"),
                "defmethod" => Some("method"),
                _ => None,
            };

            if let (Some(kind), Some(name_node)) = (kind, children.get(1)) {
                let name = text(*name_node, self.source).to_owned();
                let mut qualified_name = if self.namespace.is_empty() {
                    name.clone()
                } else {
                    format!("{}/{}", self.namespace, name)
                };
                // A defmethod shares its dispatch function name with the
                // defmulti. Preserve the dispatch value in its identity so a
                // call trace can show the concrete implementation rather
                // than collapsing all methods onto the multimethod symbol.
                if head == "defmethod" {
                    if let Some(dispatch) = children.get(2) {
                        qualified_name.push_str("::");
                        qualified_name
                            .push_str(text(*dispatch, self.source).trim_start_matches(':'));
                    }
                }
                let index = self.symbols.len();
                self.symbols.push(LocalSymbol {
                    name,
                    qualified_name,
                    kind: kind.to_owned(),
                    start_line: node.start_position().row as u32 + 1,
                    end_line: node.end_position().row as u32 + 1,
                    signature: clojure_signature(node, self.source),
                    parent: None,
                });
                let callable = if kind == "var" {
                    current_callable
                } else {
                    Some(index)
                };
                for child in children.into_iter().skip(2) {
                    self.walk(child, callable);
                }
                return;
            }

            if let Some(caller) = current_callable {
                if is_callable_symbol(head) {
                    self.calls.push(LocalCall {
                        caller,
                        name: head.rsplit('/').next().unwrap_or(head).to_owned(),
                        line: node.start_position().row as u32 + 1,
                        column: node.start_position().column as u32,
                        receiver_is_instance: false,
                    });
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.walk(child, current_callable);
        }
    }
}

fn named_children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

fn is_callable_symbol(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with(':')
        && !matches!(
            value,
            "if" | "if-not"
                | "when"
                | "when-not"
                | "let"
                | "letfn"
                | "fn"
                | "do"
                | "loop"
                | "recur"
                | "quote"
                | "var"
                | "throw"
                | "try"
                | "catch"
                | "finally"
                | "new"
                | "."
        )
}

fn clojure_signature(node: Node<'_>, source: &str) -> String {
    let raw = text(node, source);
    let first_body_line = raw.lines().take(2).collect::<Vec<_>>().join(" ");
    compact_signature(&first_body_line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_namespace_functions_and_calls() {
        let source = r#"
            (ns dev.connectome.core)
            (defn decorate [name] (str "Hi " name))
            (defn greet [name] (decorate name))
        "#;
        let file = extract("core.clj".into(), source).unwrap();
        assert_eq!(
            file.symbols
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>(),
            ["decorate", "greet"]
        );
        assert_eq!(file.symbols[1].qualified_name, "dev.connectome.core/greet");
        assert!(file.calls.iter().any(|call| call.name == "decorate"));
    }

    #[test]
    fn gives_multimethod_implementations_distinct_names() {
        let file = extract(
            "core.clj".into(),
            "(ns dev.connectome.core) (defmulti render :type) (defmethod render :html [x] (decorate x))",
        )
        .unwrap();
        assert!(file
            .symbols
            .iter()
            .any(|symbol| symbol.qualified_name == "dev.connectome.core/render::html"));
    }
}
