#!/usr/bin/env python3
"""Clone the standard benchmark repositories at their immutable pinned commits."""

from __future__ import annotations

import json
import shutil
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parent
FIXTURE_ROOT = ROOT / "fixtures"
SLICE_ROOT = FIXTURE_ROOT / "slices"


def run(*command: str, cwd: Path | None = None) -> str:
    completed = subprocess.run(command, cwd=cwd, check=True, text=True, stdout=subprocess.PIPE)
    return completed.stdout.strip()


def prepare(name: str, config: dict[str, str]) -> None:
    target = FIXTURE_ROOT / name
    commit = config["commit"]
    if not target.exists():
        print(f"Cloning {name} at {commit[:12]}")
        run("git", "clone", "--depth", "1", "--no-tags", config["url"], str(target))
        run("git", "checkout", "--detach", commit, cwd=target)
        return
    if not (target / ".git").exists():
        raise SystemExit(f"Fixture path exists but is not a Git checkout: {target}")
    actual_remote = run("git", "remote", "get-url", "origin", cwd=target)
    if actual_remote != config["url"]:
        raise SystemExit(f"Fixture remote differs for {target}: {actual_remote}")
    # Connectome deliberately writes its local .connectome index inside the
    # fixture. Only tracked-file changes make the pinned source unsafe to reuse.
    status = run("git", "status", "--porcelain", "--untracked-files=no", cwd=target)
    if status:
        raise SystemExit(f"Fixture is dirty and will not be modified: {target}")
    actual = run("git", "rev-parse", "HEAD", cwd=target)
    if actual != commit:
        print(f"Updating {name} to {commit[:12]}")
        run("git", "fetch", "--depth", "1", "origin", commit, cwd=target)
        run("git", "checkout", "--detach", commit, cwd=target)
    else:
        print(f"Using {name} at pinned commit {commit[:12]}")


def copy_slice(repository: str, source_root: Path, destination_root: Path, files: list[tuple[str, str]]) -> None:
    for source_relative, destination_relative in files:
        source = source_root / source_relative
        destination = destination_root / destination_relative
        if not source.is_file():
            raise SystemExit(f"Missing fixed fixture source: {source}")
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, destination)
    (destination_root / ".fixture.json").write_text(
        json.dumps({"repository": repository, "source_root": str(source_root), "files": files}, indent=2)
        + "\n"
    )


def prepare_slices() -> None:
    spring = FIXTURE_ROOT / "spring-boot"
    kit = FIXTURE_ROOT / "kit"
    flutter = FIXTURE_ROOT / "flutter-view"
    javascript = FIXTURE_ROOT / "algorithms-javascript"
    typescript = FIXTURE_ROOT / "algorithms-typescript"
    copy_slice(
        "spring-boot",
        spring,
        SLICE_ROOT / "spring-boot",
        [
            (
                "core/spring-boot/src/main/java/org/springframework/boot/SpringApplication.java",
                "src/main/java/org/springframework/boot/SpringApplication.java",
            )
        ],
    )
    copy_slice(
        "flutter-view",
        flutter,
        SLICE_ROOT / "flutter-view",
        [("examples/flutter_view/lib/main.dart", "lib/main.dart")],
    )
    copy_slice(
        "TheAlgorithms/JavaScript Data-Structures",
        javascript,
        SLICE_ROOT / "algorithms-javascript",
        [
            ("Data-Structures/Linked-List/SinglyLinkedList.js", "linked-list/SinglyLinkedList.js"),
            ("Data-Structures/Linked-List/ReverseSinglyLinkedList.js", "linked-list/ReverseSinglyLinkedList.js"),
        ],
    )
    copy_slice(
        "TheAlgorithms/TypeScript search",
        typescript,
        SLICE_ROOT / "algorithms-typescript",
        [
            ("search/exponential_search.ts", "search/exponential_search.ts"),
            ("search/binary_search.ts", "search/binary_search.ts"),
        ],
    )
    copy_slice(
        "connectome",
        ROOT.parent,
        SLICE_ROOT / "connectome-rust",
        [
            ("src/indexer.rs", "src/indexer.rs"),
            ("src/languages/mod.rs", "src/languages/mod.rs"),
            ("src/languages/generic.rs", "src/languages/generic.rs"),
            ("src/lsp.rs", "src/lsp.rs"),
            ("src/model.rs", "src/model.rs"),
        ],
    )
    copy_slice(
        "kit",
        kit,
        SLICE_ROOT / "kit",
        [
            ("libs/kit-generator/src/kit/generator/io.clj", "src/kit/generator/io.clj"),
            ("libs/kit-generator/src/kit/generator/renderer.clj", "src/kit/generator/renderer.clj"),
            ("libs/kit-generator/src/kit/generator/modules.clj", "src/kit/generator/modules.clj"),
            ("libs/kit-generator/src/kit/generator/modules/generator.clj", "src/kit/generator/modules/generator.clj"),
            ("libs/kit-generator/src/kit/generator/modules/injections.clj", "src/kit/generator/modules/injections.clj"),
        ],
    )


def main() -> None:
    FIXTURE_ROOT.mkdir(exist_ok=True)
    fixtures = json.loads((ROOT / "fixtures.json").read_text())
    for name, config in fixtures.items():
        prepare(name, config)
    prepare_slices()


if __name__ == "__main__":
    main()
