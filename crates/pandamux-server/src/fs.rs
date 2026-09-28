use std::fs;
use std::path::{Path, PathBuf};

use pandamux_protocol::{FsEntry, FsListResult, FsReadResult, RpcError};

/// Maximum file size permitted for reading in a single request (5 MB).
pub const MAX_READ_SIZE_BYTES: u64 = 5 * 1024 * 1024;

/// Verifies that a target path is strictly confined within `canonical_root`.
/// Returns the canonicalized path if confinement is satisfied, or an RpcError otherwise.
pub fn verify_confinement(root_path: &Path, relative_or_target: &str) -> Result<PathBuf, RpcError> {
    let canonical_root = fs::canonicalize(root_path).map_err(|e| {
        RpcError::invalid_params(format!(
            "Invalid workspace root '{}': {e}",
            root_path.display()
        ))
    })?;

    // Disallow absolute target paths that do not share the root prefix
    let target = if Path::new(relative_or_target).is_absolute() {
        PathBuf::from(relative_or_target)
    } else {
        // Strip leading slashes to prevent join from replacing root
        let clean = relative_or_target.trim_start_matches(['/', '\\']);
        canonical_root.join(clean)
    };

    // Canonicalize the target path to resolve all `..`, `.`, and symlinks
    let canonical_target = fs::canonicalize(&target).map_err(|e| {
        RpcError::invalid_params(format!(
            "Path '{}' cannot be resolved under root: {e}",
            target.display()
        ))
    })?;

    // Strict confinement assertion
    if !canonical_target.starts_with(&canonical_root) {
        return Err(RpcError::invalid_params(
            "Access denied: path traverses outside workspace root confinement",
        ));
    }

    Ok(canonical_target)
}

/// Reads a file under strict workspace root confinement.
pub fn read_confined_file(root_path: &str, file_path: &str) -> Result<FsReadResult, RpcError> {
    let root = Path::new(root_path);
    let canonical_target = verify_confinement(root, file_path)?;

    let metadata = fs::symlink_metadata(&canonical_target).map_err(|e| {
        RpcError::internal_error(format!(
            "Failed to read metadata for '{}': {e}",
            canonical_target.display()
        ))
    })?;

    if metadata.is_dir() {
        return Err(RpcError::invalid_params(format!(
            "Path '{}' is a directory, not a file",
            file_path
        )));
    }

    let size_bytes = metadata.len();
    if size_bytes > MAX_READ_SIZE_BYTES {
        return Err(RpcError::invalid_params(format!(
            "File size ({} bytes) exceeds the maximum allowed read cap ({} bytes)",
            size_bytes, MAX_READ_SIZE_BYTES
        )));
    }

    let bytes = fs::read(&canonical_target).map_err(|e| {
        RpcError::internal_error(format!(
            "Failed to read file '{}': {e}",
            canonical_target.display()
        ))
    })?;

    let is_binary = bytes.contains(&0);
    let content = if is_binary {
        String::new()
    } else {
        String::from_utf8_lossy(&bytes).to_string()
    };

    let canonical_root = fs::canonicalize(root)
        .map_err(|e| RpcError::invalid_params(format!("Invalid root '{root_path}': {e}")))?;
    let relative_path = canonical_target
        .strip_prefix(&canonical_root)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| file_path.to_string());

    Ok(FsReadResult {
        relative_path,
        content,
        is_binary,
        size_bytes,
    })
}

/// Lists files and directories in a directory under strict workspace root confinement.
pub fn list_confined_dir(root_path: &str, subpath: Option<&str>) -> Result<FsListResult, RpcError> {
    let root = Path::new(root_path);
    let target_sub = subpath.unwrap_or("");
    let canonical_target = verify_confinement(root, target_sub)?;

    let metadata = fs::metadata(&canonical_target)
        .map_err(|e| RpcError::internal_error(format!("Failed to read metadata: {e}")))?;

    if !metadata.is_dir() {
        return Err(RpcError::invalid_params(format!(
            "Path '{}' is not a directory",
            target_sub
        )));
    }

    let canonical_root = fs::canonicalize(root)
        .map_err(|e| RpcError::invalid_params(format!("Invalid root '{root_path}': {e}")))?;

    let relative_path = canonical_target
        .strip_prefix(&canonical_root)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    let mut entries = Vec::new();
    let read_dir = fs::read_dir(&canonical_target)
        .map_err(|e| RpcError::internal_error(format!("Failed to list directory: {e}")))?;

    for item in read_dir {
        let entry = match item {
            Ok(e) => e,
            Err(_) => continue,
        };

        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };

        let is_symlink = file_type.is_symlink();
        let is_dir = file_type.is_dir();
        let size_bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
        let name = entry.file_name().to_string_lossy().to_string();

        let item_path = entry.path();
        let entry_rel = item_path
            .strip_prefix(&canonical_root)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| name.clone());

        entries.push(FsEntry {
            name,
            relative_path: entry_rel,
            is_dir,
            is_symlink,
            size_bytes,
        });
    }

    // Sort directories first, then alphabetical by name
    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });

    Ok(FsListResult {
        root_path: root_path.to_string(),
        relative_path,
        entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_read_confined_file_success() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let file = root.join("hello.txt");
        fs::write(&file, "Hello confined world!").unwrap();

        let res = read_confined_file(root.to_str().unwrap(), "hello.txt").unwrap();
        assert_eq!(res.relative_path, "hello.txt");
        assert_eq!(res.content, "Hello confined world!");
        assert!(!res.is_binary);
        assert_eq!(res.size_bytes, 21);
    }

    #[test]
    fn test_read_confined_file_rejects_parent_traversal() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("workspace");
        fs::create_dir(&root).unwrap();

        // Put secret outside root
        let outside_file = dir.path().join("secret.txt");
        fs::write(&outside_file, "super secret").unwrap();

        let err = read_confined_file(root.to_str().unwrap(), "../secret.txt").unwrap_err();
        assert!(
            err.message
                .contains("traverses outside workspace root confinement")
                || err.message.contains("cannot be resolved under root"),
            "Expected confinement rejection, got: {}",
            err.message
        );
    }

    #[test]
    fn test_read_confined_file_rejects_symlink_outside_root() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("workspace");
        fs::create_dir(&root).unwrap();

        // Target outside root
        let outside_file = dir.path().join("outside.txt");
        fs::write(&outside_file, "confidential outside").unwrap();

        // Symlink inside workspace pointing to outside
        let symlink_path = root.join("link_to_outside.txt");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside_file, &symlink_path).unwrap();
        #[cfg(windows)]
        let _ = std::os::windows::fs::symlink_file(&outside_file, &symlink_path);

        if symlink_path.exists() || symlink_path.is_symlink() {
            let err =
                read_confined_file(root.to_str().unwrap(), "link_to_outside.txt").unwrap_err();
            assert!(
                err.message
                    .contains("traverses outside workspace root confinement"),
                "Expected symlink confinement rejection, got: {}",
                err.message
            );
        }
    }

    #[test]
    fn test_read_confined_file_allows_symlink_inside_root() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let target_file = root.join("target.txt");
        fs::write(&target_file, "inside target").unwrap();

        let symlink_path = root.join("link_inside.txt");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target_file, &symlink_path).unwrap();
        #[cfg(windows)]
        let _ = std::os::windows::fs::symlink_file(&target_file, &symlink_path);

        if symlink_path.exists() {
            let res = read_confined_file(root.to_str().unwrap(), "link_inside.txt").unwrap();
            assert_eq!(res.content, "inside target");
        }
    }

    #[test]
    fn test_list_confined_dir() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::create_dir(root.join("src")).unwrap();
        fs::write(root.join("src").join("main.rs"), "fn main() {}").unwrap();
        fs::write(root.join("README.md"), "# Hello").unwrap();

        let res = list_confined_dir(root.to_str().unwrap(), None).unwrap();
        assert_eq!(res.entries.len(), 2);
        // Directories first
        assert_eq!(res.entries[0].name, "src");
        assert!(res.entries[0].is_dir);
        assert_eq!(res.entries[1].name, "README.md");
        assert!(!res.entries[1].is_dir);

        // List subpath
        let sub_res = list_confined_dir(root.to_str().unwrap(), Some("src")).unwrap();
        assert_eq!(sub_res.entries.len(), 1);
        assert_eq!(sub_res.entries[0].name, "main.rs");
    }
}
