use pandamux_boundary_check::{load_metadata, validate_workspace};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const TARGET: &str = "x86_64-unknown-linux-gnu";

struct ScratchWorkspace(PathBuf);

impl ScratchWorkspace {
    fn copy_fixture(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should follow the Unix epoch")
            .as_nanos();
        let destination = std::env::temp_dir().join(format!(
            "pandamux-boundary-{name}-{}-{nonce}",
            std::process::id()
        ));
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join(name);
        copy_tree(&source, &destination);
        Self(destination)
    }

    fn manifest(&self) -> PathBuf {
        self.0.join("Cargo.toml")
    }
}

impl Drop for ScratchWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn fixture_workspaces_fail_their_named_boundary() {
    let cases = [
        ("transitive-gpui-server", "gpui.desktop_only"),
        ("default-grid-leak", "term.server_features"),
        (
            "orchestrator-native-embedding",
            "orchestrator.forbidden_dependency",
        ),
        ("gpui-version-skew", "gpui.snapshot_skew"),
    ];

    for (fixture, expected_code) in cases {
        let scratch = ScratchWorkspace::copy_fixture(fixture);
        let metadata = load_metadata(&scratch.manifest(), TARGET, false)
            .unwrap_or_else(|error| panic!("fixture {fixture} metadata failed: {error}"));
        let violations = validate_workspace(&metadata, TARGET, false);
        assert!(
            violations
                .iter()
                .any(|violation| violation.code == expected_code),
            "fixture {fixture} did not produce {expected_code}: {violations:#?}"
        );
    }
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("fixture destination should be created");
    for entry in fs::read_dir(source).expect("fixture directory should be readable") {
        let entry = entry.expect("fixture entry should be readable");
        let target = destination.join(entry.file_name());
        if entry
            .file_type()
            .expect("fixture entry type should be readable")
            .is_dir()
        {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("fixture file should be copied");
        }
    }
}
