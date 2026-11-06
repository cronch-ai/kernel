//! The kernel's layers, and a check that each module depends only on its own layer and those
//! below. The design is in the platform's `docs/2026-11-06-layers.md`.
//!
//! A module is a file or directory directly under `src/`. A dependency is any `crate::` path in
//! its code, comments excluded, so a doc comment that names another module is no dependency.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

/// The layers, lowest first. A module may depend on modules in its own layer or a lower one.
const LAYERS: &[(&str, &[&str])] = &[
    // Terms, types, checking and keys.
    ("language", &["ir", "types", "ops", "check", "cbor", "key"]),
    // The interpreter and the monitors.
    (
        "semantics",
        &["labeled", "harness", "run", "view", "property"],
    ),
    // Verdicts, obligations and their queries.
    (
        "verification",
        &["smtlib", "induction", "rules", "evidence"],
    ),
    // Requests in CBOR.
    ("wire", &["api"]),
];

fn layer_of(module: &str) -> Option<usize> {
    LAYERS
        .iter()
        .position(|(_, modules)| modules.contains(&module))
}

/// `source` without its comments. Strings are kept, so a `//` inside one ends the line early:
/// that can only hide a dependency on the rest of that line, never invent one.
fn code(source: &str) -> String {
    let mut out = String::new();
    let mut rest = source;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("/*") {
            rest = after.find("*/").map_or("", |end| &after[end + 2..]);
        } else if rest.starts_with("//") {
            rest = rest.find('\n').map_or("", |end| &rest[end..]);
        } else {
            let c = rest.chars().next().unwrap();
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

/// The modules `source` names through `crate::` paths.
fn dependencies(source: &str) -> BTreeSet<String> {
    let code = code(source);
    code.match_indices("crate::")
        .filter(|(at, _)| {
            let before = code[..*at].chars().next_back();
            !before.is_some_and(|c| c.is_alphanumeric() || c == '_')
        })
        .map(|(at, _)| {
            code[at + "crate::".len()..]
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect::<String>()
        })
        .filter(|name| !name.is_empty())
        .collect()
}

/// Every module under `src/`, with the modules it depends on.
fn graph() -> BTreeMap<String, BTreeSet<String>> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut graph = BTreeMap::new();
    let mut files = vec![src.clone()];
    while let Some(path) = files.pop() {
        if path.is_dir() {
            files.extend(fs::read_dir(&path).unwrap().map(|e| e.unwrap().path()));
            continue;
        }
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let relative = path.strip_prefix(&src).unwrap();
        let first = relative
            .components()
            .next()
            .unwrap()
            .as_os_str()
            .to_str()
            .unwrap();
        let module = first.trim_end_matches(".rs").to_string();
        if module == "lib" {
            continue;
        }
        let deps: BTreeSet<String> = dependencies(&fs::read_to_string(&path).unwrap())
            .into_iter()
            .filter(|d| *d != module)
            .collect();
        graph
            .entry(module)
            .or_insert_with(BTreeSet::new)
            .extend(deps);
    }
    graph
}

#[test]
fn every_module_has_a_layer() {
    let graph = graph();
    let unplaced: Vec<_> = graph.keys().filter(|m| layer_of(m).is_none()).collect();
    assert!(
        unplaced.is_empty(),
        "modules with no layer in LAYERS: {unplaced:?}"
    );
    let missing: Vec<_> = LAYERS
        .iter()
        .flat_map(|(_, modules)| modules.iter())
        .filter(|m| !graph.contains_key(**m))
        .collect();
    assert!(
        missing.is_empty(),
        "LAYERS names modules that don't exist: {missing:?}"
    );
}

#[test]
fn no_module_depends_on_a_higher_layer() {
    let mut upward = Vec::new();
    for (module, deps) in graph() {
        let Some(own) = layer_of(&module) else {
            continue;
        };
        for dep in deps {
            if layer_of(&dep).is_some_and(|theirs| theirs > own) {
                upward.push(format!(
                    "{module} ({}) -> {dep} ({})",
                    LAYERS[own].0,
                    LAYERS[layer_of(&dep).unwrap()].0
                ));
            }
        }
    }
    assert!(
        upward.is_empty(),
        "dependencies on a higher layer:\n{}",
        upward.join("\n")
    );
}

#[test]
fn modules_form_no_cycle() {
    let graph = graph();
    // Kahn's algorithm: whatever can't be ordered is on a cycle.
    let mut remaining = graph.clone();
    loop {
        let free: Vec<String> = remaining
            .iter()
            .filter(|(_, deps)| deps.iter().all(|d| !remaining.contains_key(d)))
            .map(|(m, _)| m.clone())
            .collect();
        if free.is_empty() {
            break;
        }
        for m in free {
            remaining.remove(&m);
        }
    }
    assert!(
        remaining.is_empty(),
        "modules on a cycle: {:?}",
        remaining.keys()
    );
}

#[test]
fn comments_are_no_dependency() {
    let source = "//! See `crate::rules`.\n/// And crate::wire.\n/* crate::api */\nuse crate::ir::{Expr, Type};\nfn f() { crate::types::zero(); notcrate::x(); }\n";
    let deps: Vec<_> = dependencies(source).into_iter().collect();
    assert_eq!(deps, ["ir", "types"]);
}
