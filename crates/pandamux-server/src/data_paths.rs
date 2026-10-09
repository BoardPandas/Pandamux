use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataPaths {
    root: PathBuf,
    data: PathBuf,
    database: PathBuf,
    backups: PathBuf,
    runtime: PathBuf,
    personal: PathBuf,
    instructions: PathBuf,
    configuration_candidates: PathBuf,
    bundles: PathBuf,
    bundle_staging: PathBuf,
    recovery: PathBuf,
    attachments: PathBuf,
}

impl DataPaths {
    pub fn from_root(root: impl Into<PathBuf>) -> io::Result<Self> {
        let root = absolute_clean(root.into())?;
        let data = root.join("data");
        Ok(Self {
            database: data.join("pandamux.db"),
            backups: data.join("backups"),
            runtime: root.join("run"),
            personal: root.join("personal"),
            instructions: root.join("instructions"),
            configuration_candidates: root.join("configuration-candidates"),
            bundles: root.join("bundles"),
            bundle_staging: root.join("bundle-staging"),
            recovery: root.join("recovery"),
            attachments: data.join("attachments"),
            root,
            data,
        })
    }

    pub fn default_for_current_user() -> io::Result<Self> {
        Self::from_root(default_app_data_root())
    }

    pub fn with_database_path(mut self, database: impl Into<PathBuf>) -> io::Result<Self> {
        let database = absolute_clean(database.into())?;
        if !database.starts_with(&self.root) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "database path must remain inside the injected app-data root",
            ));
        }
        self.database = database;
        Ok(self)
    }

    pub fn ensure_private_roots(&self) -> io::Result<()> {
        ensure_private_directory(&self.root, None)?;
        let owner_root = &self.root;
        for path in [
            &self.data,
            &self.backups,
            &self.runtime,
            &self.personal,
            &self.instructions,
            &self.configuration_candidates,
            &self.bundles,
            &self.bundle_staging,
            &self.recovery,
            &self.attachments,
        ] {
            ensure_private_directory(path, Some(owner_root))?;
        }
        let database_parent = self.database.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "database path has no parent")
        })?;
        ensure_private_directory(database_parent, Some(owner_root))?;
        Ok(())
    }

    pub fn owns(&self, path: &Path) -> bool {
        absolute_clean(path.to_path_buf()).is_ok_and(|path| path.starts_with(&self.root))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn data(&self) -> &Path {
        &self.data
    }

    pub fn database(&self) -> &Path {
        &self.database
    }

    pub fn backups(&self) -> &Path {
        &self.backups
    }

    pub fn runtime(&self) -> &Path {
        &self.runtime
    }

    pub fn personal(&self) -> &Path {
        &self.personal
    }

    pub fn instructions(&self) -> &Path {
        &self.instructions
    }

    pub fn configuration_candidates(&self) -> &Path {
        &self.configuration_candidates
    }

    pub fn bundles(&self) -> &Path {
        &self.bundles
    }

    pub fn bundle_staging(&self) -> &Path {
        &self.bundle_staging
    }

    pub fn recovery(&self) -> &Path {
        &self.recovery
    }

    pub fn attachments(&self) -> &Path {
        &self.attachments
    }
}

fn default_app_data_root() -> PathBuf {
    if let Ok(dir) = std::env::var("PANDAMUX_DATA_DIR") {
        PathBuf::from(dir)
    } else if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        PathBuf::from(local_app_data).join("pandamux")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".pandamux")
    } else {
        std::env::temp_dir().join(format!("pandamux-{}", std::process::id()))
    }
}

fn absolute_clean(path: PathBuf) -> io::Result<PathBuf> {
    let path = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()?.join(path)
    };
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "managed paths must be absolute and lexically normalized",
        ));
    }
    Ok(path)
}

fn ensure_private_directory(path: &Path, owner_root: Option<&Path>) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            reject_link_or_non_directory(path, &metadata)?;
            validate_owner(path, &metadata, owner_root)?;
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            #[cfg(windows)]
            crate::windows_security::create_private_directory(path)?;
            #[cfg(not(windows))]
            fs::create_dir_all(path)?;
            let metadata = fs::symlink_metadata(path)?;
            reject_link_or_non_directory(path, &metadata)?;
            validate_owner(path, &metadata, owner_root)?;
        }
        Err(error) => return Err(error),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn reject_link_or_non_directory(path: &Path, metadata: &fs::Metadata) -> io::Result<()> {
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "managed path is not a private directory: {}",
                path.display()
            ),
        ));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("managed path is a reparse point: {}", path.display()),
            ));
        }
    }
    Ok(())
}

fn validate_owner(
    path: &Path,
    metadata: &fs::Metadata,
    owner_root: Option<&Path>,
) -> io::Result<()> {
    #[cfg(unix)]
    if let Some(owner_root) = owner_root {
        use std::os::unix::fs::MetadataExt;
        let root_metadata = fs::symlink_metadata(owner_root)?;
        if metadata.uid() != root_metadata.uid() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("managed path has unsafe ownership: {}", path.display()),
            ));
        }
    }
    #[cfg(not(unix))]
    let _ = (metadata, owner_root);
    #[cfg(windows)]
    crate::windows_security::validate_private_directory(path)?;
    #[cfg(not(any(unix, windows)))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_graph_matches_the_trusted_configuration_layout() {
        let temp = tempfile::tempdir().expect("temporary root");
        let paths = DataPaths::from_root(temp.path().join("app")).expect("data paths");
        assert_eq!(paths.database(), temp.path().join("app/data/pandamux.db"));
        assert_eq!(paths.personal(), temp.path().join("app/personal"));
        assert_eq!(paths.bundles(), temp.path().join("app/bundles"));
        assert_eq!(
            paths.bundle_staging(),
            temp.path().join("app/bundle-staging")
        );
        paths.ensure_private_roots().expect("private roots");
        paths
            .ensure_private_roots()
            .expect("private roots remain valid after reopen");
        assert!(paths.backups().is_dir());
        assert!(paths.recovery().is_dir());
        assert!(paths.owns(&paths.configuration_candidates().join("candidate")));
        assert!(!paths.owns(temp.path().join("outside").as_path()));
    }

    #[cfg(unix)]
    #[test]
    fn root_rejects_symlink_substitution() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("temporary root");
        let target = temp.path().join("target");
        fs::create_dir(&target).expect("target");
        let linked = temp.path().join("linked");
        symlink(&target, &linked).expect("symlink");
        let paths = DataPaths::from_root(linked).expect("data paths");
        assert_eq!(
            paths
                .ensure_private_roots()
                .expect_err("symlink must fail")
                .kind(),
            io::ErrorKind::PermissionDenied
        );
    }

    #[test]
    fn database_override_cannot_escape_root() {
        let temp = tempfile::tempdir().expect("temporary root");
        let paths = DataPaths::from_root(temp.path().join("app")).expect("data paths");
        assert!(
            paths
                .with_database_path(temp.path().join("outside.db"))
                .is_err()
        );
    }

    #[cfg(windows)]
    #[test]
    fn root_rejects_junction_substitution() {
        use std::os::windows::process::CommandExt;
        use std::process::Command;

        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let temp = tempfile::tempdir().expect("temporary root");
        let target = temp.path().join("target");
        fs::create_dir(&target).expect("junction target");
        let linked = temp.path().join("linked");
        let output = Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(&linked)
            .arg(&target)
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .expect("create test junction");
        assert!(
            output.status.success(),
            "junction creation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let paths = DataPaths::from_root(&linked).expect("data paths");
        assert_eq!(
            paths
                .ensure_private_roots()
                .expect_err("junction must fail")
                .kind(),
            io::ErrorKind::PermissionDenied
        );
        fs::remove_dir(&linked).expect("remove test junction");
    }
}
