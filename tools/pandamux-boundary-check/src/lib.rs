#![forbid(unsafe_code)]

use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

pub const SUPPORTED_TARGETS: [&str; 5] = [
    "x86_64-pc-windows-msvc",
    "aarch64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-musl",
];

const PURE_ORCHESTRATOR_FORBIDDEN_EXACT: [&str; 19] = [
    "alacritty_terminal",
    "fastembed",
    "gpui",
    "keyring",
    "libsqlite3-sys",
    "oo7",
    "ort",
    "ort-sys",
    "pandamux-desktop",
    "pandamux-memory",
    "pandamux-providers",
    "pandamux-remote",
    "pandamux-server",
    "pandamux-term",
    "portable-pty",
    "reqwest",
    "rusqlite",
    "russh",
    "tokio",
];

const EMBEDDING_STACK_PREFIXES: [&str; 8] = [
    "candle-",
    "fastembed",
    "hnsw",
    "onnxruntime",
    "ort",
    "sqlite-vss",
    "tokenizers",
    "usearch",
];

const SINGLE_VERSION_NATIVE_STACKS: [&str; 12] = [
    "alacritty_terminal",
    "aws-lc-sys",
    "libgit2-sys",
    "libsqlite3-sys",
    "libssh2-sys",
    "onnxruntime-sys",
    "openssl-sys",
    "ort-sys",
    "portable-pty",
    "ring",
    "rocksdb-sys",
    "zstd-sys",
];

#[derive(Debug, Deserialize)]
pub struct Metadata {
    packages: Vec<Package>,
    workspace_members: Vec<String>,
    resolve: Option<Resolve>,
}

#[derive(Clone, Debug, Deserialize)]
struct Package {
    id: String,
    name: String,
    version: String,
    dependencies: Vec<Dependency>,
    features: BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Debug, Deserialize)]
struct Dependency {
    name: String,
    source: Option<String>,
    req: String,
    rename: Option<String>,
    kind: Option<String>,
    optional: bool,
    uses_default_features: bool,
    features: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Resolve {
    nodes: Vec<Node>,
}

#[derive(Debug, Deserialize)]
struct Node {
    id: String,
    deps: Vec<NodeDependency>,
}

#[derive(Debug, Deserialize)]
struct NodeDependency {
    name: String,
    pkg: String,
    dep_kinds: Vec<DependencyKind>,
}

#[derive(Debug, Deserialize)]
struct DependencyKind {
    kind: Option<String>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Violation {
    pub target: String,
    pub code: &'static str,
    pub message: String,
}

impl Violation {
    pub fn render(&self) -> String {
        format!("{} [{}]: {}", self.code, self.target, self.message)
    }
}

#[derive(Default)]
struct RootResolution {
    reachable: BTreeSet<String>,
    features: BTreeMap<String, BTreeSet<String>>,
    parents: BTreeMap<String, String>,
}

struct Graph<'a> {
    packages: BTreeMap<&'a str, &'a Package>,
    nodes: BTreeMap<&'a str, &'a Node>,
    workspace_members: BTreeSet<&'a str>,
}

impl<'a> Graph<'a> {
    fn new(metadata: &'a Metadata) -> Result<Self, String> {
        let resolve = metadata
            .resolve
            .as_ref()
            .ok_or_else(|| "cargo metadata did not return a resolve graph".to_string())?;
        Ok(Self {
            packages: metadata
                .packages
                .iter()
                .map(|package| (package.id.as_str(), package))
                .collect(),
            nodes: resolve
                .nodes
                .iter()
                .map(|node| (node.id.as_str(), node))
                .collect(),
            workspace_members: metadata
                .workspace_members
                .iter()
                .map(String::as_str)
                .collect(),
        })
    }

    fn workspace_package(&self, name: &str) -> Option<&'a Package> {
        self.workspace_members.iter().find_map(|id| {
            self.packages
                .get(id)
                .copied()
                .filter(|package| package.name == name)
        })
    }

    fn resolved_packages(&self) -> impl Iterator<Item = &'a Package> + '_ {
        self.nodes
            .keys()
            .filter_map(|id| self.packages.get(id).copied())
    }

    fn resolve_root(&self, root: &Package) -> RootResolution {
        let mut resolution = RootResolution::default();
        resolution.reachable.insert(root.id.clone());
        resolution
            .features
            .entry(root.id.clone())
            .or_default()
            .insert("default".to_string());

        let mut queue = VecDeque::from([root.id.clone()]);
        while let Some(package_id) = queue.pop_front() {
            let Some(package) = self.packages.get(package_id.as_str()).copied() else {
                continue;
            };
            let Some(node) = self.nodes.get(package_id.as_str()).copied() else {
                continue;
            };
            let seeds = resolution
                .features
                .get(package_id.as_str())
                .cloned()
                .unwrap_or_default();
            let expanded = expand_features(package, &seeds);
            resolution
                .features
                .insert(package_id.clone(), expanded.local.clone());

            for dependency in package
                .dependencies
                .iter()
                .filter(|dependency| dependency.kind.as_deref() != Some("dev"))
            {
                let alias = dependency.rename.as_deref().unwrap_or(&dependency.name);
                let resolved_alias = alias.replace('-', "_");
                let Some(edge) = node.deps.iter().find(|edge| {
                    edge.name == resolved_alias
                        && edge
                            .dep_kinds
                            .iter()
                            .any(|kind| kind.kind.as_deref() != Some("dev"))
                }) else {
                    continue;
                };
                let enabled = !dependency.optional || expanded.enabled_dependencies.contains(alias);
                if !enabled {
                    continue;
                }

                let mut requested = BTreeSet::new();
                if dependency.uses_default_features {
                    requested.insert("default".to_string());
                }
                requested.extend(dependency.features.iter().cloned());
                if let Some(forwarded) = expanded.dependency_features.get(alias) {
                    requested.extend(forwarded.iter().cloned());
                }

                let was_reachable = resolution.reachable.insert(edge.pkg.clone());
                if was_reachable {
                    resolution
                        .parents
                        .insert(edge.pkg.clone(), package_id.clone());
                }
                let active = resolution.features.entry(edge.pkg.clone()).or_default();
                let before = active.len();
                active.extend(requested);
                if was_reachable || active.len() != before {
                    queue.push_back(edge.pkg.clone());
                }
            }
        }

        resolution
    }

    fn package_name(&self, id: &str) -> &str {
        self.packages
            .get(id)
            .map(|package| package.name.as_str())
            .unwrap_or("<unknown package>")
    }

    fn path_names(&self, root: &Package, target_id: &str, resolution: &RootResolution) -> String {
        let mut ids = vec![target_id.to_string()];
        let mut cursor = target_id;
        while cursor != root.id {
            let Some(parent) = resolution.parents.get(cursor) else {
                break;
            };
            ids.push(parent.clone());
            cursor = parent;
        }
        ids.reverse();
        ids.iter()
            .map(|id| self.package_name(id))
            .collect::<Vec<_>>()
            .join(" -> ")
    }
}

#[derive(Default)]
struct ExpandedFeatures {
    local: BTreeSet<String>,
    enabled_dependencies: BTreeSet<String>,
    dependency_features: BTreeMap<String, BTreeSet<String>>,
}

fn expand_features(package: &Package, seeds: &BTreeSet<String>) -> ExpandedFeatures {
    let mut expanded = ExpandedFeatures {
        local: seeds.clone(),
        ..ExpandedFeatures::default()
    };
    let mut queue: VecDeque<String> = seeds.iter().cloned().collect();

    while let Some(feature) = queue.pop_front() {
        let Some(entries) = package.features.get(&feature) else {
            continue;
        };
        for entry in entries {
            if let Some(alias) = entry.strip_prefix("dep:") {
                expanded.enabled_dependencies.insert(alias.to_string());
                continue;
            }
            if let Some((alias, dependency_feature)) = entry.split_once("?/") {
                expanded
                    .dependency_features
                    .entry(alias.to_string())
                    .or_default()
                    .insert(dependency_feature.to_string());
                continue;
            }
            if let Some((alias, dependency_feature)) = entry.split_once('/') {
                expanded.enabled_dependencies.insert(alias.to_string());
                expanded
                    .dependency_features
                    .entry(alias.to_string())
                    .or_default()
                    .insert(dependency_feature.to_string());
                continue;
            }
            if expanded.local.insert(entry.clone()) {
                queue.push_back(entry.clone());
            }
        }
    }

    for dependency in package
        .dependencies
        .iter()
        .filter(|dependency| dependency.optional)
    {
        let alias = dependency.rename.as_deref().unwrap_or(&dependency.name);
        if expanded.local.contains(alias) {
            expanded.enabled_dependencies.insert(alias.to_string());
        }
    }
    expanded
}

pub fn load_metadata(manifest_path: &Path, target: &str, locked: bool) -> Result<Metadata, String> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let mut command = Command::new(cargo);
    command
        .arg("metadata")
        .arg("--format-version=1")
        .arg("--manifest-path")
        .arg(manifest_path)
        .arg("--filter-platform")
        .arg(target);
    if locked {
        command.arg("--locked");
    }
    let output = command
        .output()
        .map_err(|error| format!("failed to start cargo metadata: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("cargo metadata returned invalid JSON: {error}"))
}

pub fn validate_workspace(
    metadata: &Metadata,
    target: &str,
    require_current_shape: bool,
) -> Vec<Violation> {
    let graph = match Graph::new(metadata) {
        Ok(graph) => graph,
        Err(message) => {
            return vec![violation(target, "boundary.resolve_missing", message)];
        }
    };
    let mut violations = Vec::new();

    if require_current_shape {
        for required in [
            "pandamux-core",
            "pandamux-orchestrator",
            "pandamux-protocol",
            "pandamux-server",
            "pandamux-desktop",
            "pandamux-term",
        ] {
            if graph.workspace_package(required).is_none() {
                violations.push(violation(
                    target,
                    "workspace.required_member_missing",
                    format!("required workspace member {required} is absent"),
                ));
            }
        }
    }

    let resolutions: BTreeMap<&str, RootResolution> = graph
        .workspace_members
        .iter()
        .filter_map(|id| graph.packages.get(id).copied())
        .map(|package| (package.name.as_str(), graph.resolve_root(package)))
        .collect();

    enforce_direct_dependency_pins(&graph, target, &mut violations);
    enforce_gpui_ownership(&graph, &resolutions, target, &mut violations);
    enforce_gpui_snapshot(&graph, target, &mut violations);
    enforce_terminal_features(&graph, &resolutions, target, &mut violations);
    enforce_orchestrator_purity(&graph, &resolutions, target, &mut violations);
    enforce_memory_isolation(&graph, &resolutions, target, &mut violations);
    enforce_native_stack_versions(&graph, target, &mut violations);

    violations
}

fn enforce_direct_dependency_pins(
    graph: &Graph<'_>,
    target: &str,
    violations: &mut Vec<Violation>,
) {
    for package_id in &graph.workspace_members {
        let Some(package) = graph.packages.get(package_id).copied() else {
            continue;
        };
        for dependency in &package.dependencies {
            let Some(source) = dependency.source.as_deref() else {
                continue;
            };
            if source.starts_with("registry+") && !is_exact_requirement(&dependency.req) {
                violations.push(violation(
                    target,
                    "dependency.direct_range",
                    format!(
                        "{} depends on registry package {} with non-exact requirement {}; move it to an exact workspace pin or add a reviewed policy exception",
                        package.name, dependency.name, dependency.req
                    ),
                ));
            }
        }
    }
}

fn is_exact_requirement(requirement: &str) -> bool {
    requirement
        .strip_prefix('=')
        .is_some_and(|version| !version.is_empty() && !version.contains(','))
}

fn enforce_gpui_ownership(
    graph: &Graph<'_>,
    resolutions: &BTreeMap<&str, RootResolution>,
    target: &str,
    violations: &mut Vec<Violation>,
) {
    for (root_name, resolution) in resolutions {
        if *root_name == "pandamux-desktop" {
            continue;
        }
        let Some(root) = graph.workspace_package(root_name) else {
            continue;
        };
        for package_id in &resolution.reachable {
            let package_name = graph.package_name(package_id);
            if is_gpui_package(package_name) {
                violations.push(violation(
                    target,
                    "gpui.desktop_only",
                    format!(
                        "{} reaches GPUI through {}",
                        root_name,
                        graph.path_names(root, package_id, resolution)
                    ),
                ));
            }
        }
    }
}

fn enforce_gpui_snapshot(graph: &Graph<'_>, target: &str, violations: &mut Vec<Violation>) {
    for package in graph.resolved_packages() {
        let expected = if package.name == "gpui-pre-reqwest" {
            Some("0.12.15")
        } else if package.name == "gpui-pre" || package.name.starts_with("gpui-pre-") {
            Some("0.3.6")
        } else if matches!(
            package.name.as_str(),
            "gpui-base"
                | "gpui-component"
                | "gpui-component-macros"
                | "gpui-kit"
                | "gpui-kit-assets"
        ) {
            Some("0.6.6")
        } else {
            None
        };
        if package.name == "gpui" {
            violations.push(violation(
                target,
                "gpui.stale_crate",
                format!(
                    "stale crates.io gpui {} is resolved; PandaMUX uses the gpui-pre snapshot",
                    package.version
                ),
            ));
        }
        if let Some(expected) = expected
            && package.version != expected
        {
            violations.push(violation(
                target,
                "gpui.snapshot_skew",
                format!(
                    "{} resolved at {}, expected the deliberate snapshot {}",
                    package.name, package.version, expected
                ),
            ));
        }
    }
}

fn enforce_terminal_features(
    graph: &Graph<'_>,
    resolutions: &BTreeMap<&str, RootResolution>,
    target: &str,
    violations: &mut Vec<Violation>,
) {
    check_term_root_features(
        graph,
        resolutions,
        target,
        "pandamux-server",
        &["pty"],
        "term.server_features",
        violations,
    );
    check_term_root_features(
        graph,
        resolutions,
        target,
        "pandamux-desktop",
        &["grid"],
        "term.desktop_features",
        violations,
    );

    for (root_name, resolution) in resolutions {
        let Some(root) = graph.workspace_package(root_name) else {
            continue;
        };
        for package_id in &resolution.reachable {
            let package_name = graph.package_name(package_id);
            let forbidden = (package_name == "alacritty_terminal"
                && !matches!(*root_name, "pandamux-desktop" | "pandamux-term"))
                || (package_name == "portable-pty"
                    && !matches!(*root_name, "pandamux-server" | "pandamux-term"));
            if forbidden {
                violations.push(violation(
                    target,
                    "term.implementation_leak",
                    format!(
                        "{} reaches {} through {}",
                        root_name,
                        package_name,
                        graph.path_names(root, package_id, resolution)
                    ),
                ));
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn check_term_root_features(
    graph: &Graph<'_>,
    resolutions: &BTreeMap<&str, RootResolution>,
    target: &str,
    root_name: &str,
    expected: &[&str],
    code: &'static str,
    violations: &mut Vec<Violation>,
) {
    let Some(resolution) = resolutions.get(root_name) else {
        return;
    };
    let Some(term) = graph
        .packages
        .values()
        .copied()
        .find(|package| package.name == "pandamux-term")
    else {
        return;
    };
    let actual = resolution
        .features
        .get(&term.id)
        .cloned()
        .unwrap_or_default();
    let expected: BTreeSet<String> = expected.iter().map(|value| (*value).to_string()).collect();
    if actual != expected {
        violations.push(violation(
            target,
            code,
            format!(
                "{root_name} resolves pandamux-term features {:?}; expected {:?}",
                actual, expected
            ),
        ));
    }
}

fn enforce_orchestrator_purity(
    graph: &Graph<'_>,
    resolutions: &BTreeMap<&str, RootResolution>,
    target: &str,
    violations: &mut Vec<Violation>,
) {
    let Some(root) = graph.workspace_package("pandamux-orchestrator") else {
        return;
    };
    let Some(resolution) = resolutions.get("pandamux-orchestrator") else {
        return;
    };
    for package_id in &resolution.reachable {
        if package_id == &root.id {
            continue;
        }
        let package_name = graph.package_name(package_id);
        if PURE_ORCHESTRATOR_FORBIDDEN_EXACT.contains(&package_name)
            || is_gpui_package(package_name)
            || EMBEDDING_STACK_PREFIXES
                .iter()
                .any(|prefix| package_name == *prefix || package_name.starts_with(prefix))
        {
            violations.push(violation(
                target,
                "orchestrator.forbidden_dependency",
                format!(
                    "pandamux-orchestrator reaches forbidden dependency through {}",
                    graph.path_names(root, package_id, resolution)
                ),
            ));
        }
    }
}

fn enforce_memory_isolation(
    graph: &Graph<'_>,
    resolutions: &BTreeMap<&str, RootResolution>,
    target: &str,
    violations: &mut Vec<Violation>,
) {
    for (root_name, resolution) in resolutions {
        let Some(root) = graph.workspace_package(root_name) else {
            continue;
        };
        for package_id in &resolution.reachable {
            let package_name = graph.package_name(package_id);
            if !EMBEDDING_STACK_PREFIXES
                .iter()
                .any(|prefix| package_name == *prefix || package_name.starts_with(prefix))
            {
                continue;
            }
            let path = graph.path_names(root, package_id, resolution);
            if !path.split(" -> ").any(|name| name == "pandamux-memory") {
                violations.push(violation(
                    target,
                    "memory.embedding_isolation",
                    format!("embedding stack bypasses pandamux-memory through {path}"),
                ));
            }
        }
    }
}

fn enforce_native_stack_versions(graph: &Graph<'_>, target: &str, violations: &mut Vec<Violation>) {
    let mut versions: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for package in graph.resolved_packages() {
        if SINGLE_VERSION_NATIVE_STACKS.contains(&package.name.as_str()) {
            versions
                .entry(package.name.as_str())
                .or_default()
                .insert(package.version.as_str());
        }
    }
    for (name, versions) in versions {
        if versions.len() > 1 {
            violations.push(violation(
                target,
                "native.duplicate_stack",
                format!("{name} resolves multiple native-stack versions: {versions:?}"),
            ));
        }
    }
}

fn is_gpui_package(name: &str) -> bool {
    name == "gpui"
        || name == "gpui-base"
        || name == "gpui-component"
        || name == "gpui-component-macros"
        || name == "gpui-kit"
        || name == "gpui-kit-assets"
        || name == "gpui-pre"
        || name.starts_with("gpui-pre-")
}

fn violation(target: &str, code: &'static str, message: String) -> Violation {
    Violation {
        target: target.to_string(),
        code,
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feature_expansion_tracks_optional_dependency_and_forwarded_features() {
        let package = Package {
            id: "root".to_string(),
            name: "root".to_string(),
            version: "1.0.0".to_string(),
            dependencies: vec![Dependency {
                name: "child".to_string(),
                source: None,
                req: "*".to_string(),
                rename: None,
                kind: None,
                optional: true,
                uses_default_features: false,
                features: Vec::new(),
            }],
            features: BTreeMap::from([(
                "grid".to_string(),
                vec!["dep:child".to_string(), "child/render".to_string()],
            )]),
        };
        let expanded = expand_features(&package, &BTreeSet::from(["grid".to_string()]));
        assert!(expanded.enabled_dependencies.contains("child"));
        assert_eq!(
            expanded.dependency_features["child"],
            BTreeSet::from(["render".to_string()])
        );
    }

    #[test]
    fn exact_requirement_rejects_compatible_ranges() {
        assert!(is_exact_requirement("=1.2.3"));
        assert!(is_exact_requirement("=1.2.3-beta.1"));
        assert!(!is_exact_requirement("1.2.3"));
        assert!(!is_exact_requirement("^1.2.3"));
        assert!(!is_exact_requirement(">=1.2, <2"));
        assert!(!is_exact_requirement("*"));
    }
}
