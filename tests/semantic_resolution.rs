use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn copy_tree(source: &Path, target: &Path) {
    fs::create_dir_all(target).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let destination = target.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &destination);
        } else {
            fs::copy(entry.path(), destination).unwrap();
        }
    }
}

fn run_index(binary: &Path, root: &Path, mode: &str, commands: bool) -> Value {
    let mut command = Command::new(binary);
    command.args(["index", root.to_str().unwrap(), "--lsp-mode", mode]);
    if commands {
        let fake = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/semantic_fake_lsp.py");
        let python = if cfg!(windows) { "python" } else { "python3" };
        let value = format!("{python} {}", fake.display());
        command
            .arg("--jdtls-command")
            .arg(&value)
            .arg("--clojure-lsp-command")
            .arg(&value)
            .args(["--lsp-timeout-ms", "1000"]);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "index failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn semantic_servers_improve_ambiguous_resolution_and_fallback_cleanly() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("connectome-semantic-{nonce}"));
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/semantic");
    copy_tree(&source, &root);
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_connectome"));

    let parser_only = run_index(&binary, &root, "off", false);
    assert_eq!(parser_only["lsp"]["mode"], "off");
    assert_eq!(parser_only["calls"]["resolved"], 1);

    let warm = run_index(&binary, &root, "off", false);
    assert_eq!(warm["incremental"]["parsed"], 0);
    assert_eq!(warm["incremental"]["reused"], 5);

    let semantic = run_index(&binary, &root, "on", true);
    assert_eq!(semantic["lsp"]["resolved"], 3);
    assert!(semantic["lsp"]["capabilities"].as_array().unwrap().len() >= 4);
    assert!(semantic["lsp"]["call_hierarchy"].as_u64().unwrap() >= 1);
    assert!(semantic["calls"]["resolved"].as_u64().unwrap() >= 3);

    let fallback = {
        let mut command = Command::new(&binary);
        command.args([
            "index",
            root.to_str().unwrap(),
            "--lsp-mode",
            "on",
            "--jdtls-command",
            "missing-jdtls",
            "--clojure-lsp-command",
            "missing-clojure-lsp",
            "--lsp-timeout-ms",
            "250",
        ]);
        let output = command.output().unwrap();
        assert!(output.status.success());
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    assert!(!fallback["lsp"]["warnings"].as_array().unwrap().is_empty());
    assert_eq!(
        fallback["calls"]["resolved"],
        parser_only["calls"]["resolved"]
    );

    fs::remove_dir_all(root).unwrap();
}
