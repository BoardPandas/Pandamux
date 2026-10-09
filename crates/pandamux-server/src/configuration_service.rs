use crate::{InstructionsService, PersonalRepoService, Store, StoreError};
use pandamux_core::{
    BundleSourceManifest, CompiledInstructionBundleV1, ConfigurationRevision, ContentSha256,
    InstructionSourceId, OrganizationId, ProjectId, ProjectLayerTrustId, ProjectLayerTrustRecord,
    ProjectRepositoryId, ProjectRepositoryRecord, normalize_git_remote, parse_git_remote_url,
};
use pandamux_orchestrator::{
    CompileBundleInput, DiagnosticSet, LayerGrant, LayeredSourceDocument, SourceDocument,
    TrustedBundleSettings, compile_instruction_bundle, parse_source_document, source_layer_sha256,
    validate_source_set_bounds,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::{self, File, Metadata};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

const MAX_GIT_CONFIG_BYTES: usize = 1024 * 1024;
const MAX_GIT_POINTER_BYTES: usize = 4096;
const MAX_DISPLAY_PATH_BYTES: usize = 512;

#[derive(Clone, Debug)]
pub struct ProjectLayerSnapshot {
    pub repository: ProjectRepositoryRecord,
    pub documents: Vec<SourceDocument>,
    pub project_content_sha256: Option<ContentSha256>,
    pub trust: Option<ProjectLayerTrustRecord>,
    pub issues: Vec<Value>,
    pub visible_but_inactive: bool,
}

#[derive(Clone, Debug)]
pub struct ConfigurationCompilation {
    pub bundle: CompiledInstructionBundleV1,
    pub project_layer: ProjectLayerSnapshot,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConfigurationServiceError {
    pub reason: &'static str,
    pub safe_message: String,
    pub retryable: bool,
    pub issues: Vec<Value>,
}

impl std::fmt::Display for ConfigurationServiceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.safe_message)
    }
}

impl std::error::Error for ConfigurationServiceError {}

impl From<StoreError> for ConfigurationServiceError {
    fn from(_: StoreError) -> Self {
        state_error()
    }
}

#[derive(Clone)]
pub struct ConfigurationService {
    store: Store,
    instructions: InstructionsService,
    personal: PersonalRepoService,
}

impl ConfigurationService {
    pub fn new(
        store: Store,
        instructions: InstructionsService,
        personal: PersonalRepoService,
    ) -> Self {
        Self {
            store,
            instructions,
            personal,
        }
    }

    pub fn inspect_project_layer(
        &self,
        project_id: &ProjectId,
        workspace_root: &Path,
        inspected_at_ms: u64,
    ) -> Result<ProjectLayerSnapshot, ConfigurationServiceError> {
        let policy = self
            .store
            .organization_policy_for_project(project_id)
            .map_err(|_| ConfigurationServiceError {
                reason: "project_organization_missing",
                safe_message: "The project has no canonical organization assignment".to_string(),
                retryable: false,
                issues: Vec::new(),
            })?;
        self.inspect_project_layer_in_scope(project_id, &policy.id, workspace_root, inspected_at_ms)
    }

    pub fn trust_project_layer(
        &self,
        project_id: &ProjectId,
        workspace_root: &Path,
        expected_content_sha256: &ContentSha256,
        approved_by: &str,
        approved_at_ms: u64,
    ) -> Result<ProjectLayerTrustRecord, ConfigurationServiceError> {
        if approved_by.trim().is_empty()
            || approved_by.len() > 256
            || approved_by.chars().any(char::is_control)
        {
            return Err(ConfigurationServiceError {
                reason: "project_trust_actor_invalid",
                safe_message: "The project trust actor is invalid".to_string(),
                retryable: false,
                issues: Vec::new(),
            });
        }
        let snapshot = self.inspect_project_layer(project_id, workspace_root, approved_at_ms)?;
        if !snapshot.issues.is_empty() {
            return Err(ConfigurationServiceError {
                reason: "project_layer_invalid",
                safe_message: "The project configuration must be valid before it can be trusted"
                    .to_string(),
                retryable: false,
                issues: snapshot.issues,
            });
        }
        let Some(content_sha256) = snapshot.project_content_sha256 else {
            return Err(ConfigurationServiceError {
                reason: "project_layer_missing",
                safe_message: "The project has no configuration layer to trust".to_string(),
                retryable: false,
                issues: Vec::new(),
            });
        };
        if &content_sha256 != expected_content_sha256 {
            return Err(ConfigurationServiceError {
                reason: "project_content_changed",
                safe_message: "The project configuration changed before trust was recorded"
                    .to_string(),
                retryable: true,
                issues: Vec::new(),
            });
        }
        if let Some(existing) = self.store.get_active_project_layer_trust(
            &snapshot.repository.organization_id,
            &snapshot.repository.project_id,
            &snapshot.repository.id,
            &content_sha256,
        )? {
            return Ok(existing);
        }
        let trust = ProjectLayerTrustRecord {
            id: ProjectLayerTrustId::generate(),
            organization_id: snapshot.repository.organization_id,
            project_id: snapshot.repository.project_id,
            repository_id: snapshot.repository.id,
            project_content_sha256: content_sha256,
            approved_by: approved_by.to_string(),
            approved_at_ms,
            revoked_at_ms: None,
        };
        self.store.insert_project_layer_trust(&trust)?;
        Ok(trust)
    }

    pub fn revoke_project_layer_trust(
        &self,
        project_id: &ProjectId,
        revoked_at_ms: u64,
    ) -> Result<bool, ConfigurationServiceError> {
        let policy = self.store.organization_policy_for_project(project_id)?;
        let repository = self
            .store
            .get_project_repository(project_id, &policy.id)?
            .ok_or_else(|| ConfigurationServiceError {
                reason: "project_repository_missing",
                safe_message: "The project repository identity has not been discovered".to_string(),
                retryable: false,
                issues: Vec::new(),
            })?;
        Ok(self
            .store
            .revoke_project_layer_trust(&repository.id, revoked_at_ms)?
            > 0)
    }

    pub fn compile_for_project(
        &self,
        project_id: &ProjectId,
        workspace_root: &Path,
        instruction_source_id: &InstructionSourceId,
        compiled_at_ms: u64,
    ) -> Result<ConfigurationCompilation, ConfigurationServiceError> {
        let project_layer =
            self.inspect_project_layer(project_id, workspace_root, compiled_at_ms)?;
        let (instruction_source, instruction_documents) = self
            .instructions
            .active_snapshot(instruction_source_id)
            .map_err(|error| ConfigurationServiceError {
                reason: error.reason,
                safe_message: error.safe_message,
                retryable: error.retryable,
                issues: Vec::new(),
            })?
            .ok_or_else(|| ConfigurationServiceError {
                reason: "active_instructions_missing",
                safe_message: "No active instructions snapshot is available".to_string(),
                retryable: false,
                issues: Vec::new(),
            })?;
        let instructions_commit_sha = instruction_source
            .active_source_commit_sha
            .clone()
            .ok_or_else(state_error)?;
        let (personal_commit_sha, personal_documents) =
            self.personal
                .current_documents()
                .map_err(|error| ConfigurationServiceError {
                    reason: error.reason,
                    safe_message: error.safe_message,
                    retryable: error.retryable,
                    issues: Vec::new(),
                })?;
        let settings = self.store.get_or_initialize_settings(compiled_at_ms)?;
        let tier_mapping =
            settings
                .settings
                .tier_mapping
                .clone()
                .ok_or_else(|| ConfigurationServiceError {
                    reason: "trusted_tier_mapping_missing",
                    safe_message: "Trusted settings do not define an agent tier mapping"
                        .to_string(),
                    retryable: false,
                    issues: Vec::new(),
                })?;

        let mut documents = instruction_documents
            .iter()
            .cloned()
            .map(|source| LayeredSourceDocument {
                layer: pandamux_core::ValueLayer::Instructions,
                source,
            })
            .collect::<Vec<_>>();
        let instructions_hash = source_layer_sha256(&instruction_documents);
        let mut grants = vec![LayerGrant {
            layer: pandamux_core::ValueLayer::Instructions,
            trusted: true,
            content_sha256: instructions_hash,
            organization_id: project_layer.repository.organization_id.clone(),
            project_id: None,
        }];

        let trusted_project_hash = if project_layer.trust.is_some()
            && project_layer.issues.is_empty()
            && !project_layer.documents.is_empty()
        {
            let hash = project_layer.project_content_sha256.clone();
            documents.extend(project_layer.documents.iter().cloned().map(|source| {
                LayeredSourceDocument {
                    layer: pandamux_core::ValueLayer::Project,
                    source,
                }
            }));
            if let Some(hash) = &hash {
                grants.push(LayerGrant {
                    layer: pandamux_core::ValueLayer::Project,
                    trusted: true,
                    content_sha256: hash.clone(),
                    organization_id: project_layer.repository.organization_id.clone(),
                    project_id: Some(project_id.clone()),
                });
            }
            hash
        } else {
            None
        };

        if !personal_documents.is_empty() {
            let personal_hash = source_layer_sha256(&personal_documents);
            documents.extend(personal_documents.iter().cloned().map(|source| {
                LayeredSourceDocument {
                    layer: pandamux_core::ValueLayer::Personal,
                    source,
                }
            }));
            grants.push(LayerGrant {
                layer: pandamux_core::ValueLayer::Personal,
                trusted: true,
                content_sha256: personal_hash,
                organization_id: project_layer.repository.organization_id.clone(),
                project_id: Some(project_id.clone()),
            });
        }

        let input = CompileBundleInput {
            documents,
            grants,
            source_manifest: BundleSourceManifest {
                instructions_commit_sha,
                project_layer_content_sha256: trusted_project_hash,
                personal_commit_sha: Some(personal_commit_sha),
                settings_revision: settings.revision,
            },
            project_id: project_id.clone(),
            organization_id: project_layer.repository.organization_id.clone(),
            settings: TrustedBundleSettings {
                revision: settings.revision,
                tier_mapping,
                disabled_agent_ids: settings
                    .settings
                    .trusted_configuration
                    .disabled_agent_ids
                    .clone(),
                full_access_agent_ids: settings
                    .settings
                    .trusted_configuration
                    .full_access_agent_ids
                    .clone(),
            },
        };
        let bundle = compile_instruction_bundle(&input).map_err(diagnostics_error)?;
        Ok(ConfigurationCompilation {
            bundle,
            project_layer,
        })
    }

    fn inspect_project_layer_in_scope(
        &self,
        project_id: &ProjectId,
        organization_id: &OrganizationId,
        workspace_root: &Path,
        inspected_at_ms: u64,
    ) -> Result<ProjectLayerSnapshot, ConfigurationServiceError> {
        let identity = discover_repository(workspace_root)?;
        let existing = self
            .store
            .get_project_repository(project_id, organization_id)?;
        let repository = match existing {
            Some(existing)
                if existing.canonical_remote_identity == identity.canonical_remote_identity
                    && existing.canonical_repository_root_identity
                        == identity.canonical_repository_root_identity
                    && existing.display_path == identity.display_path =>
            {
                existing
            }
            Some(mut existing) => {
                existing.canonical_remote_identity = identity.canonical_remote_identity.clone();
                existing.canonical_repository_root_identity =
                    identity.canonical_repository_root_identity.clone();
                existing.display_path = identity.display_path.clone();
                existing.revision = existing
                    .revision
                    .checked_next()
                    .map_err(|_| state_error())?;
                existing.updated_at_ms = inspected_at_ms;
                self.store.save_project_repository(&existing)?;
                existing
            }
            None => {
                let repository = ProjectRepositoryRecord {
                    id: ProjectRepositoryId::generate(),
                    project_id: project_id.clone(),
                    organization_id: organization_id.clone(),
                    canonical_remote_identity: identity.canonical_remote_identity.clone(),
                    canonical_repository_root_identity: identity
                        .canonical_repository_root_identity
                        .clone(),
                    display_path: identity.display_path.clone(),
                    revision: ConfigurationRevision::INITIAL,
                    created_at_ms: inspected_at_ms,
                    updated_at_ms: inspected_at_ms,
                };
                self.store.save_project_repository(&repository)?;
                repository
            }
        };
        let (documents, mut issues) = collect_project_documents(&identity.repository_root)?;
        for document in &documents {
            if let Err(error) = parse_source_document(document) {
                issues.extend(
                    error
                        .into_issues()
                        .into_iter()
                        .filter_map(|issue| serde_json::to_value(issue).ok()),
                );
            }
        }
        if let Err(error) = validate_source_set_bounds(&documents) {
            issues.extend(
                error
                    .into_issues()
                    .into_iter()
                    .filter_map(|issue| serde_json::to_value(issue).ok()),
            );
        }
        sort_issues(&mut issues);
        let project_content_sha256 =
            (!documents.is_empty()).then(|| source_layer_sha256(&documents));
        let trust = match &project_content_sha256 {
            Some(hash) if issues.is_empty() => self.store.get_active_project_layer_trust(
                organization_id,
                project_id,
                &repository.id,
                hash,
            )?,
            _ => None,
        };
        let visible_but_inactive =
            project_content_sha256.is_some() && (trust.is_none() || !issues.is_empty());
        Ok(ProjectLayerSnapshot {
            repository,
            documents,
            project_content_sha256,
            trust,
            issues,
            visible_but_inactive,
        })
    }
}

struct RepositoryIdentity {
    repository_root: PathBuf,
    canonical_remote_identity: String,
    canonical_repository_root_identity: String,
    display_path: String,
}

fn discover_repository(
    workspace_root: &Path,
) -> Result<RepositoryIdentity, ConfigurationServiceError> {
    if !workspace_root.is_absolute()
        || workspace_root
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(repository_error(
            "The project workspace path is not canonical",
        ));
    }
    reject_link_components(workspace_root)?;
    let mut repository_root = None;
    for ancestor in workspace_root.ancestors().take(128) {
        let git = ancestor.join(".git");
        if fs::symlink_metadata(&git).is_ok() {
            repository_root = Some(ancestor.to_path_buf());
            break;
        }
    }
    let repository_root = repository_root
        .ok_or_else(|| repository_error("The project workspace is not inside a Git repository"))?;
    let git_entry = repository_root.join(".git");
    reject_link_components(&git_entry)?;
    let metadata = fs::symlink_metadata(&git_entry)
        .map_err(|_| repository_error("The project Git identity could not be inspected"))?;
    let git_dir = if metadata.is_dir() && !is_link_or_reparse(&metadata) {
        fs::canonicalize(&git_entry)
            .map_err(|_| repository_error("The project Git identity could not be inspected"))?
    } else if metadata.is_file() && !is_link_or_reparse(&metadata) {
        let pointer = read_bounded_file(&git_entry, MAX_GIT_POINTER_BYTES)?;
        let pointer = std::str::from_utf8(&pointer)
            .map_err(|_| repository_error("The project Git pointer is invalid"))?;
        let relative = pointer
            .trim()
            .strip_prefix("gitdir:")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| repository_error("The project Git pointer is invalid"))?;
        let path = if Path::new(relative).is_absolute() {
            PathBuf::from(relative)
        } else {
            repository_root.join(relative)
        };
        reject_link_components(&path)?;
        fs::canonicalize(path)
            .map_err(|_| repository_error("The project Git pointer is invalid"))?
    } else {
        return Err(repository_error("The project Git identity is unsafe"));
    };
    let common_dir = resolve_common_git_dir(&git_dir)?;
    reject_link_components(&common_dir)?;
    let config_path = common_dir.join("config");
    reject_link_components(&config_path)?;
    let config = read_bounded_file(&config_path, MAX_GIT_CONFIG_BYTES)?;
    let config = std::str::from_utf8(&config)
        .map_err(|_| repository_error("The project Git configuration is invalid"))?;
    let remote = parse_git_remote_url(config)
        .as_deref()
        .and_then(normalize_git_remote)
        .filter(|remote| {
            !remote.is_empty() && remote.len() <= 4096 && !remote.chars().any(char::is_control)
        })
        .ok_or_else(|| {
            repository_error("The project repository has no canonical credential-free remote")
        })?;
    let canonical_common_dir = fs::canonicalize(&common_dir)
        .map_err(|_| repository_error("The project Git identity could not be inspected"))?;
    let root_identity = format!(
        "git-common-dir-sha256:{}",
        sha256_hex(canonical_common_dir.to_string_lossy().as_bytes())
    );
    Ok(RepositoryIdentity {
        display_path: safe_path(&repository_root),
        repository_root,
        canonical_remote_identity: remote,
        canonical_repository_root_identity: root_identity,
    })
}

fn resolve_common_git_dir(git_dir: &Path) -> Result<PathBuf, ConfigurationServiceError> {
    let common_pointer = git_dir.join("commondir");
    if !common_pointer.exists() {
        return Ok(git_dir.to_path_buf());
    }
    reject_link_components(&common_pointer)?;
    let pointer = read_bounded_file(&common_pointer, MAX_GIT_POINTER_BYTES)?;
    let pointer = std::str::from_utf8(&pointer)
        .map_err(|_| repository_error("The project common Git pointer is invalid"))?
        .trim();
    if pointer.is_empty() {
        return Err(repository_error(
            "The project common Git pointer is invalid",
        ));
    }
    let path = if Path::new(pointer).is_absolute() {
        PathBuf::from(pointer)
    } else {
        git_dir.join(pointer)
    };
    fs::canonicalize(path)
        .map_err(|_| repository_error("The project common Git pointer is invalid"))
}

fn collect_project_documents(
    repository_root: &Path,
) -> Result<(Vec<SourceDocument>, Vec<Value>), ConfigurationServiceError> {
    let root = repository_root.join(".pandamux");
    let metadata = match fs::symlink_metadata(&root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok((Vec::new(), Vec::new()));
        }
        Err(_) => return Err(project_read_error()),
    };
    if is_link_or_reparse(&metadata) || !metadata.is_dir() {
        return Ok((
            Vec::new(),
            vec![project_issue(
                "project_layer_root_unsafe",
                "The .pandamux project layer must be a real directory",
                "<root>",
            )],
        ));
    }
    let mut files = Vec::<(String, PathBuf)>::new();
    let mut issues = Vec::new();
    for entry in sorted_directory_entries(&root)? {
        let name = match entry.file_name().to_str() {
            Some(name) => name.to_string(),
            None => {
                issues.push(project_issue(
                    "configuration_path_non_utf8",
                    "A project configuration path is not valid UTF-8",
                    "<invalid>",
                ));
                continue;
            }
        };
        let metadata = fs::symlink_metadata(entry.path()).map_err(|_| project_read_error())?;
        if name == "project.yaml" {
            if !is_regular_configuration_file(&metadata) {
                issues.push(project_issue(
                    "configuration_special_mode_rejected",
                    "Project configuration files must be regular files",
                    &name,
                ));
            } else {
                files.push((name, entry.path()));
            }
            continue;
        }
        if matches!(name.as_str(), "agents" | "knowledge")
            && metadata.is_dir()
            && !is_link_or_reparse(&metadata)
        {
            collect_project_directory(&entry.path(), &name, &mut files, &mut issues)?;
            continue;
        }
        issues.push(project_issue(
            "configuration_path_rejected",
            "The project layer contains an unsupported path",
            &name,
        ));
    }
    if !files.iter().any(|(path, _)| path == "project.yaml") {
        issues.push(project_issue(
            "project_document_missing",
            "A project layer must contain project.yaml",
            "project.yaml",
        ));
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    if files.len() > pandamux_orchestrator::schema::MAX_SOURCE_FILES {
        issues.push(project_issue(
            "config_file_count_limit",
            "The project layer contains more configuration files than allowed",
            "<project>",
        ));
        files.truncate(pandamux_orchestrator::schema::MAX_SOURCE_FILES);
    }
    let mut casefolded = BTreeSet::new();
    let mut documents = Vec::new();
    let mut aggregate = 0usize;
    for (logical_path, path) in files {
        if !casefolded.insert(logical_path.to_lowercase()) {
            issues.push(project_issue(
                "configuration_path_case_collision",
                "Project configuration paths collide when compared without case",
                &logical_path,
            ));
            continue;
        }
        let limit = if logical_path == "project.yaml" {
            pandamux_orchestrator::schema::MAX_YAML_FILE_BYTES
        } else {
            pandamux_orchestrator::schema::MAX_MARKDOWN_FILE_BYTES
        };
        let bytes = match read_bounded_file(&path, limit) {
            Ok(bytes) => bytes,
            Err(error) if error.reason == "project_file_too_large" => {
                issues.push(project_issue(
                    "config_file_too_large",
                    "A project configuration file exceeds its size limit",
                    &logical_path,
                ));
                continue;
            }
            Err(error) => return Err(error),
        };
        aggregate = aggregate.saturating_add(bytes.len());
        match SourceDocument::new(logical_path.clone(), bytes) {
            Ok(document) => documents.push(document),
            Err(_) => issues.push(project_issue(
                "configuration_path_rejected",
                "The project layer contains an unsupported path",
                &logical_path,
            )),
        }
    }
    if aggregate > pandamux_orchestrator::schema::MAX_SOURCE_BYTES {
        issues.push(project_issue(
            "config_bundle_size_limit",
            "The project layer exceeds its aggregate size limit",
            "<project>",
        ));
        documents.clear();
    }
    sort_issues(&mut issues);
    Ok((documents, issues))
}

fn collect_project_directory(
    directory: &Path,
    prefix: &str,
    files: &mut Vec<(String, PathBuf)>,
    issues: &mut Vec<Value>,
) -> Result<(), ConfigurationServiceError> {
    for entry in sorted_directory_entries(directory)? {
        let name = match entry.file_name().to_str() {
            Some(name) => name.to_string(),
            None => {
                issues.push(project_issue(
                    "configuration_path_non_utf8",
                    "A project configuration path is not valid UTF-8",
                    "<invalid>",
                ));
                continue;
            }
        };
        let logical_path = format!("{prefix}/{name}");
        let metadata = fs::symlink_metadata(entry.path()).map_err(|_| project_read_error())?;
        if !is_regular_configuration_file(&metadata) || !name.ends_with(".md") {
            issues.push(project_issue(
                "configuration_path_rejected",
                "Project agent and knowledge paths must be regular Markdown files",
                &logical_path,
            ));
            continue;
        }
        files.push((logical_path, entry.path()));
    }
    Ok(())
}

fn sorted_directory_entries(path: &Path) -> Result<Vec<fs::DirEntry>, ConfigurationServiceError> {
    let mut entries = fs::read_dir(path)
        .map_err(|_| project_read_error())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| project_read_error())?;
    entries.sort_by_key(|entry| entry.file_name());
    Ok(entries)
}

fn read_bounded_file(path: &Path, max_bytes: usize) -> Result<Vec<u8>, ConfigurationServiceError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| project_read_error())?;
    if is_link_or_reparse(&metadata) || !metadata.is_file() {
        return Err(project_read_error());
    }
    if metadata.len() > max_bytes as u64 {
        return Err(ConfigurationServiceError {
            reason: "project_file_too_large",
            safe_message: "A project configuration file exceeds its size limit".to_string(),
            retryable: false,
            issues: Vec::new(),
        });
    }
    let file = File::open(path).map_err(|_| project_read_error())?;
    let opened = file.metadata().map_err(|_| project_read_error())?;
    if !opened.is_file() || opened.len() != metadata.len() {
        return Err(project_read_error());
    }
    let mut bytes = Vec::new();
    file.take(max_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| project_read_error())?;
    if bytes.len() > max_bytes {
        return Err(ConfigurationServiceError {
            reason: "project_file_too_large",
            safe_message: "A project configuration file exceeds its size limit".to_string(),
            retryable: false,
            issues: Vec::new(),
        });
    }
    Ok(bytes)
}

fn reject_link_components(path: &Path) -> Result<(), ConfigurationServiceError> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if is_link_or_reparse(&metadata) => {
                return Err(repository_error(
                    "The project repository path contains a link or reparse point",
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => {
                return Err(repository_error(
                    "The project repository path could not be inspected safely",
                ));
            }
        }
    }
    Ok(())
}

#[cfg(windows)]
fn is_link_or_reparse(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_link_or_reparse(metadata: &Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(unix)]
fn is_regular_configuration_file(metadata: &Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.is_file()
        && !is_link_or_reparse(metadata)
        && metadata.permissions().mode() & 0o111 == 0
}

#[cfg(not(unix))]
fn is_regular_configuration_file(metadata: &Metadata) -> bool {
    metadata.is_file() && !is_link_or_reparse(metadata)
}

fn project_issue(code: &str, safe_reason: &str, path: &str) -> Value {
    json!({
        "code": code,
        "safeReason": safe_reason,
        "origin": {
            "layer": "project",
            "logicalPath": safe_text(path, pandamux_orchestrator::schema::MAX_LOGICAL_PATH_BYTES),
            "field": "/",
        },
    })
}

fn sort_issues(issues: &mut Vec<Value>) {
    issues.sort_by_key(pandamux_orchestrator::canonical_json_bytes);
    issues.dedup();
    if issues.len() > pandamux_orchestrator::diagnostics::MAX_DIAGNOSTICS {
        issues.truncate(pandamux_orchestrator::diagnostics::MAX_DIAGNOSTICS);
    }
}

fn safe_path(path: &Path) -> String {
    safe_text(&path.to_string_lossy(), MAX_DISPLAY_PATH_BYTES)
}

fn safe_text(value: &str, max_bytes: usize) -> String {
    let sanitized = value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    if sanitized.len() <= max_bytes {
        return sanitized;
    }
    let mut end = max_bytes;
    while !sanitized.is_char_boundary(end) {
        end -= 1;
    }
    sanitized[..end].to_string()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut value = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut value, "{byte:02x}").expect("writing to String cannot fail");
    }
    value
}

fn diagnostics_error(error: DiagnosticSet) -> ConfigurationServiceError {
    ConfigurationServiceError {
        reason: "configuration_compilation_failed",
        safe_message: "The trusted configuration could not be compiled".to_string(),
        retryable: false,
        issues: error
            .into_issues()
            .into_iter()
            .filter_map(|issue| serde_json::to_value(issue).ok())
            .collect(),
    }
}

fn repository_error(message: &str) -> ConfigurationServiceError {
    ConfigurationServiceError {
        reason: "project_repository_identity_invalid",
        safe_message: message.to_string(),
        retryable: false,
        issues: Vec::new(),
    }
}

fn project_read_error() -> ConfigurationServiceError {
    ConfigurationServiceError {
        reason: "project_layer_read_failed",
        safe_message: "The project configuration could not be read safely".to_string(),
        retryable: true,
        issues: Vec::new(),
    }
}

fn state_error() -> ConfigurationServiceError {
    ConfigurationServiceError {
        reason: "configuration_state_failed",
        safe_message: "The trusted configuration state could not be read or persisted".to_string(),
        retryable: true,
        issues: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DataPaths;
    use pandamux_core::{BroadsidePolicy, OrganizationPolicy, ProviderInstanceId, SubscriptionKey};

    struct Fixture {
        _temp: tempfile::TempDir,
        repository: PathBuf,
        store: Store,
        service: ConfigurationService,
        project_id: ProjectId,
        organization_id: OrganizationId,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().expect("temporary configuration fixture");
            let repository = temp.path().join("repository");
            create_repository(&repository, "git@github.com:BoardPandas/Pandamux.git");
            write_project(&repository, "[]");
            let paths = DataPaths::from_root(temp.path().join("app")).expect("paths");
            paths.ensure_private_roots().expect("private roots");
            let store = Store::in_memory().expect("store");
            let project_id = ProjectId::new("project-configuration-test");
            let organization_id = OrganizationId::new("organization-configuration-test");
            store
                .save_organization_policy(
                    &OrganizationPolicy {
                        id: organization_id.clone(),
                        display_name: "Configuration test".to_string(),
                        project_ids: vec![project_id.clone()],
                        allowed_provider_instances: vec![ProviderInstanceId::new("provider-test")],
                        allowed_subscriptions: vec![SubscriptionKey::new("subscription-test")],
                        failover_pool_ids: Vec::new(),
                        default_tier_mapping: None,
                        max_concurrency: Some(1),
                        daily_budget_micro_usd: None,
                        broadside: BroadsidePolicy::default(),
                        revision: 1,
                    },
                    1,
                )
                .expect("organization policy");
            let instructions = InstructionsService::new(paths.clone(), store.clone());
            let personal = PersonalRepoService::new(paths, store.clone());
            let service = ConfigurationService::new(store.clone(), instructions, personal);
            Self {
                _temp: temp,
                repository,
                store,
                service,
                project_id,
                organization_id,
            }
        }
    }

    #[test]
    fn trust_is_exact_hash_scoped_and_identity_changes_revoke_it() {
        let fixture = Fixture::new();
        let initial = fixture
            .service
            .inspect_project_layer(&fixture.project_id, &fixture.repository, 10)
            .expect("inspect initial layer");
        assert_eq!(
            initial.repository.canonical_remote_identity,
            "github.com/boardpandas/pandamux"
        );
        assert!(initial.trust.is_none());
        assert!(initial.visible_but_inactive);
        let initial_hash = initial
            .project_content_sha256
            .clone()
            .expect("project hash");
        let trust = fixture
            .service
            .trust_project_layer(
                &fixture.project_id,
                &fixture.repository,
                &initial_hash,
                "local-control",
                11,
            )
            .expect("trust initial layer");
        let trusted = fixture
            .service
            .inspect_project_layer(&fixture.project_id, &fixture.repository, 12)
            .expect("inspect trusted layer");
        assert_eq!(
            trusted.trust.as_ref().map(|value| &value.id),
            Some(&trust.id)
        );
        assert!(!trusted.visible_but_inactive);

        write_project(&fixture.repository, "[cargo test]");
        let changed = fixture
            .service
            .inspect_project_layer(&fixture.project_id, &fixture.repository, 13)
            .expect("inspect changed content");
        assert_ne!(changed.project_content_sha256, Some(initial_hash));
        assert!(changed.trust.is_none());
        assert!(changed.visible_but_inactive);
        let changed_hash = changed
            .project_content_sha256
            .clone()
            .expect("changed hash");
        let changed_trust = fixture
            .service
            .trust_project_layer(
                &fixture.project_id,
                &fixture.repository,
                &changed_hash,
                "local-control",
                14,
            )
            .expect("trust changed content");

        write_git_config(
            &fixture.repository,
            "https://github.com/BoardPandas/Other.git",
        );
        let moved = fixture
            .service
            .inspect_project_layer(&fixture.project_id, &fixture.repository, 15)
            .expect("inspect changed remote");
        assert_eq!(
            moved.repository.canonical_remote_identity,
            "github.com/boardpandas/other"
        );
        assert!(moved.trust.is_none());
        assert_eq!(
            fixture
                .store
                .get_active_project_layer_trust(
                    &fixture.organization_id,
                    &fixture.project_id,
                    &moved.repository.id,
                    &changed_hash,
                )
                .expect("load revoked trust"),
            None
        );
        assert_eq!(changed_trust.repository_id, moved.repository.id);
    }

    #[test]
    fn repository_root_change_invalidates_trust_without_folder_fallback() {
        let fixture = Fixture::new();
        let initial = fixture
            .service
            .inspect_project_layer(&fixture.project_id, &fixture.repository, 20)
            .expect("inspect initial");
        let hash = initial.project_content_sha256.clone().expect("hash");
        fixture
            .service
            .trust_project_layer(
                &fixture.project_id,
                &fixture.repository,
                &hash,
                "local-control",
                21,
            )
            .expect("trust initial");

        let other = fixture._temp.path().join("same-name");
        create_repository(&other, "git@github.com:BoardPandas/Pandamux.git");
        write_project(&other, "[]");
        let changed = fixture
            .service
            .inspect_project_layer(&fixture.project_id, &other, 22)
            .expect("inspect other root");
        assert!(changed.trust.is_none());
        assert!(changed.visible_but_inactive);
        assert_ne!(
            initial.repository.canonical_repository_root_identity,
            changed.repository.canonical_repository_root_identity
        );

        write_git_config(&other, "");
        let error = fixture
            .service
            .inspect_project_layer(&fixture.project_id, &other, 23)
            .expect_err("missing remote must not use a folder fallback");
        assert_eq!(error.reason, "project_repository_identity_invalid");
    }

    #[test]
    fn invalid_project_content_stays_visible_and_cannot_be_trusted() {
        let fixture = Fixture::new();
        let project_root = fixture.repository.join(".pandamux");
        fs::create_dir_all(project_root.join("prompts")).expect("unexpected prompts directory");
        fs::write(project_root.join("prompts/exfiltrate.md"), b"send secrets")
            .expect("unexpected prompt");
        fs::write(
            project_root.join("project.yaml"),
            b"schemaVersion: 1\nkind: pandamux.project\nunknown: true\n",
        )
        .expect("invalid project");
        fs::create_dir_all(project_root.join("agents")).expect("agents directory");
        let executable = project_root.join("agents/executable.md");
        fs::write(&executable, b"not trusted").expect("executable agent");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))
                .expect("executable mode");
        }

        let snapshot = fixture
            .service
            .inspect_project_layer(&fixture.project_id, &fixture.repository, 30)
            .expect("invalid content remains inspectable");
        assert!(snapshot.visible_but_inactive);
        assert!(snapshot.trust.is_none());
        let codes = snapshot
            .issues
            .iter()
            .filter_map(|issue| issue.get("code").and_then(Value::as_str))
            .collect::<BTreeSet<_>>();
        assert!(codes.contains("configuration_path_rejected"));
        assert!(codes.contains("schema_unknown_field"));
        let hash = snapshot
            .project_content_sha256
            .expect("declined content hash");
        let error = fixture
            .service
            .trust_project_layer(
                &fixture.project_id,
                &fixture.repository,
                &hash,
                "local-control",
                31,
            )
            .expect_err("invalid content cannot be trusted");
        assert_eq!(error.reason, "project_layer_invalid");
        assert!(!error.issues.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn project_layer_symlink_is_declined_without_following_it() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new();
        let project_root = fixture.repository.join(".pandamux");
        fs::create_dir_all(project_root.join("agents")).expect("agents directory");
        let outside = fixture._temp.path().join("outside.md");
        fs::write(&outside, b"outside secret").expect("outside file");
        symlink(&outside, project_root.join("agents/linked.md")).expect("symlink");
        let snapshot = fixture
            .service
            .inspect_project_layer(&fixture.project_id, &fixture.repository, 40)
            .expect("symlink stays a visible issue");
        assert!(snapshot.issues.iter().any(|issue| {
            issue.get("code").and_then(Value::as_str) == Some("configuration_path_rejected")
        }));
        assert!(
            snapshot
                .documents
                .iter()
                .all(|document| document.logical_path().as_str() != "agents/linked.md")
        );
    }

    fn create_repository(path: &Path, remote: &str) {
        fs::create_dir_all(path.join(".git")).expect("git directory");
        write_git_config(path, remote);
    }

    fn write_git_config(path: &Path, remote: &str) {
        let contents = if remote.is_empty() {
            "[core]\n\trepositoryformatversion = 0\n".to_string()
        } else {
            format!(
                "[core]\n\trepositoryformatversion = 0\n[remote \"origin\"]\n\turl = {remote}\n"
            )
        };
        fs::write(path.join(".git/config"), contents).expect("git config");
    }

    fn write_project(path: &Path, verification: &str) {
        fs::create_dir_all(path.join(".pandamux")).expect("project layer directory");
        fs::write(
            path.join(".pandamux/project.yaml"),
            format!(
                "schemaVersion: 1\nkind: pandamux.project\nclassOverrides: []\nverification: {verification}\nagents: []\nknowledge: []\n"
            ),
        )
        .expect("project configuration");
    }
}
