use pandamux_boundary_check::{SUPPORTED_TARGETS, load_metadata, validate_workspace};
use std::path::Path;

fn main() {
    let manifest = Path::new("Cargo.toml");
    if !manifest.is_file() {
        eprintln!(
            "boundary.workspace_root_required: run this command from the PandaMUX workspace root"
        );
        std::process::exit(2);
    }

    let mut violations = Vec::new();
    for target in SUPPORTED_TARGETS {
        match load_metadata(manifest, target, true) {
            Ok(metadata) => violations.extend(validate_workspace(&metadata, target, true)),
            Err(error) => {
                eprintln!("boundary.metadata_failed [{target}]: {error}");
                std::process::exit(2);
            }
        }
    }

    violations.sort();
    violations.dedup();
    if violations.is_empty() {
        println!(
            "Resolved Rust dependency boundaries satisfied for {} declared targets.",
            SUPPORTED_TARGETS.len()
        );
        return;
    }

    for violation in &violations {
        eprintln!("{}", violation.render());
    }
    eprintln!(
        "Resolved Rust dependency boundary check found {} violation(s).",
        violations.len()
    );
    std::process::exit(1);
}
