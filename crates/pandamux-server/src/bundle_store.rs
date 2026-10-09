use crate::managed_git::{
    MAX_CONFIGURATION_GIT_BUNDLE_BYTES, ManagedGitCommand, ManagedGitError, ManagedGitRunner,
};
use crate::{BundleObjectRecord, DataPaths, Store, StoreError};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use pandamux_core::{
    ArchiveSha256, BundleActivationId, BundleActivationKind, BundleActivationRecord,
    BundleDistributionReceipt, BundleDistributionState, BundleSha, BundleSourceManifest,
    BundleTransferId, CompiledInstructionBundleV1, ContentSha256, EnvironmentId, SourceCommitSha,
};
use pandamux_orchestrator::{
    SourceDocument, canonical_bundle_bytes, canonical_json_bytes, parse_source_document,
    source_layer_sha256, validate_source_set_bounds, verify_compiled_bundle,
};
use pandamux_protocol::{BUNDLE_PUSH_SCHEMA_VERSION, BundlePushParams, BundlePushResult};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub const BUNDLE_ARCHIVE_SCHEMA_VERSION: u32 = 1;
pub const MAX_BUNDLE_ARCHIVE_BYTES: u64 = 96 * 1024 * 1024;

const ARCHIVE_FILE: &str = "archive.json";
const BUNDLE_FILE: &str = "compiled-bundle.json";
const INSTRUCTIONS_GIT_BUNDLE_FILE: &str = "source.bundle";
const MANIFEST_FILE: &str = "manifest.json";
const PERSONAL_GIT_BUNDLE_FILE: &str = "personal.bundle";

#[derive(Clone, Debug)]
pub struct BundleMaterializationInput {
    pub bundle: CompiledInstructionBundleV1,
    pub instructions_documents: Vec<SourceDocument>,
    pub project_documents: Vec<SourceDocument>,
    pub personal_documents: Vec<SourceDocument>,
    pub instructions_ref: String,
    pub instructions_git_bundle: Vec<u8>,
    pub personal_ref: String,
    pub personal_git_bundle: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterializedBundle {
    pub object: BundleObjectRecord,
    pub archive_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundleStoreError {
    pub reason: &'static str,
    pub safe_message: String,
    pub retryable: bool,
}

impl std::fmt::Display for BundleStoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.safe_message)
    }
}

impl std::error::Error for BundleStoreError {}

impl From<StoreError> for BundleStoreError {
    fn from(_: StoreError) -> Self {
        state_error()
    }
}

impl From<ManagedGitError> for BundleStoreError {
    fn from(error: ManagedGitError) -> Self {
        Self {
            reason: error.reason,
            safe_message: error.safe_message,
            retryable: error.retryable,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArchivedDocument {
    logical_path: String,
    bytes_base64: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BundleArchiveV1 {
    schema_version: u32,
    bundle_sha: BundleSha,
    source_manifest: BundleSourceManifest,
    compiler_version: String,
    compiled_bundle: CompiledInstructionBundleV1,
    instructions_ref: String,
    instructions_git_bundle_base64: String,
    personal_ref: String,
    personal_git_bundle_base64: String,
    instructions_documents: Vec<ArchivedDocument>,
    project_documents: Vec<ArchivedDocument>,
    personal_documents: Vec<ArchivedDocument>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BundleObjectManifestV1 {
    schema_version: u32,
    bundle_sha: BundleSha,
    source_manifest: BundleSourceManifest,
    compiler_version: String,
    compiled_bundle_sha256: ContentSha256,
    instructions_git_bundle_sha256: ContentSha256,
    personal_git_bundle_sha256: ContentSha256,
    archive_sha256: ArchiveSha256,
    archive_bytes: u64,
}

#[derive(Clone)]
pub struct BundleStore {
    paths: DataPaths,
    store: Store,
    git: ManagedGitRunner,
    owner: Arc<Mutex<()>>,
}

impl BundleStore {
    pub fn new(paths: DataPaths, store: Store) -> Self {
        Self {
            git: ManagedGitRunner::new(paths.clone()),
            paths,
            store,
            owner: Arc::new(Mutex::new(())),
        }
    }

    pub fn shutdown(&self) {
        self.git.shutdown();
    }

    pub fn materialize(
        &self,
        input: &BundleMaterializationInput,
        verified_at_ms: u64,
    ) -> Result<MaterializedBundle, BundleStoreError> {
        let _owner = self.owner.lock().expect("bundle store owner lock");
        self.paths
            .ensure_private_roots()
            .map_err(|_| materialization_error())?;
        let archive = archive_from_input(input)?;
        let archive_bytes = canonical_json_bytes(&archive);
        if archive_bytes.len() as u64 > MAX_BUNDLE_ARCHIVE_BYTES {
            return Err(integrity_error(
                "The compiled bundle archive exceeds its size limit",
            ));
        }
        self.verify_archive(&archive, None)?;
        self.install_archive(&archive, &archive_bytes, verified_at_ms)
    }

    pub fn prepare_upload(
        &self,
        transfer_id: &BundleTransferId,
    ) -> Result<PathBuf, BundleStoreError> {
        let _owner = self.owner.lock().expect("bundle store owner lock");
        self.paths
            .ensure_private_roots()
            .map_err(|_| materialization_error())?;
        let path = self.incoming_path(transfer_id);
        if path.exists() {
            let metadata = fs::symlink_metadata(&path).map_err(|_| materialization_error())?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(materialization_error());
            }
            fs::remove_file(&path).map_err(|_| materialization_error())?;
        }
        Ok(path)
    }

    pub fn discard_upload(&self, transfer_id: &BundleTransferId) -> Result<(), BundleStoreError> {
        let _owner = self.owner.lock().expect("bundle store owner lock");
        let path = self.incoming_path(transfer_id);
        if !path.exists() {
            return Ok(());
        }
        let metadata = fs::symlink_metadata(&path).map_err(|_| materialization_error())?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(materialization_error());
        }
        fs::remove_file(path).map_err(|_| materialization_error())
    }

    pub fn receive_uploaded_bundle(
        &self,
        environment_id: &EnvironmentId,
        params: &BundlePushParams,
        received_at_ms: u64,
    ) -> Result<BundlePushResult, BundleStoreError> {
        if params.schema_version != BUNDLE_PUSH_SCHEMA_VERSION {
            return Err(integrity_error(
                "The bundle transfer schema or environment does not match this node",
            ));
        }
        let expected_path = self.incoming_path(&params.transfer_id);
        let supplied_path = PathBuf::from(&params.private_artifact_ref);
        if supplied_path != expected_path {
            return Err(integrity_error(
                "The bundle transfer does not reference its private staged artifact",
            ));
        }

        let _owner = self.owner.lock().expect("bundle store owner lock");
        let archive_bytes = read_private_bounded_file(&expected_path, MAX_BUNDLE_ARCHIVE_BYTES)?;
        if archive_bytes.len() as u64 != params.archive_bytes
            || sha256_hex(&archive_bytes) != params.archive_sha256.as_str()
        {
            return Err(integrity_error(
                "The uploaded bundle archive does not match its transfer identity",
            ));
        }
        let archive: BundleArchiveV1 = serde_json::from_slice(&archive_bytes)
            .map_err(|_| integrity_error("The uploaded bundle archive is not valid"))?;
        if archive.bundle_sha != params.bundle_sha
            || archive.source_manifest != params.source_manifest
            || archive.compiler_version != params.compiler_version
        {
            return Err(integrity_error(
                "The uploaded bundle metadata does not match its transfer manifest",
            ));
        }
        self.verify_archive(&archive, Some(environment_id))?;
        let materialized = self.install_archive(&archive, &archive_bytes, received_at_ms)?;
        let activation = BundleActivationRecord {
            id: BundleActivationId::generate(),
            bundle_sha: params.bundle_sha.clone(),
            source_manifest: params.source_manifest.clone(),
            activated_at_ms: received_at_ms,
            kind: BundleActivationKind::Recovery,
            actor: "authenticated-hub".to_string(),
            prior_bundle_sha: params.expected_prior_bundle_sha.clone(),
        };
        let receipt = BundleDistributionReceipt {
            transfer_id: params.transfer_id.clone(),
            environment_id: environment_id.clone(),
            bundle_sha: params.bundle_sha.clone(),
            archive_sha256: params.archive_sha256.clone(),
            stored_bundle_sha: params.bundle_sha.clone(),
            active_bundle_sha: params.bundle_sha.clone(),
            prior_bundle_sha: params.expected_prior_bundle_sha.clone(),
            state: BundleDistributionState::Active,
            received_at_ms,
        };
        let receipt =
            self.store
                .accept_received_bundle(&materialized.object, &activation, &receipt)?;
        let _ = fs::remove_file(&expected_path);
        Ok(BundlePushResult {
            transfer_id: receipt.transfer_id,
            stored_bundle_sha: receipt.stored_bundle_sha,
            active_bundle_sha: receipt.active_bundle_sha,
            prior_bundle_sha: receipt.prior_bundle_sha,
            received_at_ms: receipt.received_at_ms,
            status: receipt.state,
        })
    }

    pub fn archive_path(&self, object: &BundleObjectRecord) -> Result<PathBuf, BundleStoreError> {
        let expected_relative = format!("bundles/{}", object.bundle_sha.as_str());
        if object.object_path != expected_relative {
            return Err(integrity_error(
                "The bundle object path does not match its content identity",
            ));
        }
        let path = self.object_path(&object.bundle_sha).join(ARCHIVE_FILE);
        let bytes = read_private_bounded_file(&path, MAX_BUNDLE_ARCHIVE_BYTES)?;
        if bytes.len() as u64 != object.archive_bytes
            || sha256_hex(&bytes) != object.archive_sha256.as_str()
        {
            return Err(integrity_error(
                "The stored bundle archive failed its integrity check",
            ));
        }
        Ok(path)
    }

    pub fn verify_active_object(&self) -> Result<Option<MaterializedBundle>, BundleStoreError> {
        let Some(activation) = self.store.current_bundle_activation()? else {
            return Ok(None);
        };
        let object = self
            .store
            .get_bundle_object(&activation.bundle_sha)?
            .ok_or_else(|| integrity_error("The active bundle object is missing"))?;
        let archive_path = self.archive_path(&object)?;
        let bytes = read_private_bounded_file(&archive_path, MAX_BUNDLE_ARCHIVE_BYTES)?;
        let archive: BundleArchiveV1 = serde_json::from_slice(&bytes)
            .map_err(|_| integrity_error("The active bundle archive is invalid"))?;
        self.verify_archive(&archive, None)?;
        Ok(Some(MaterializedBundle {
            object,
            archive_path,
        }))
    }

    fn install_archive(
        &self,
        archive: &BundleArchiveV1,
        archive_bytes: &[u8],
        verified_at_ms: u64,
    ) -> Result<MaterializedBundle, BundleStoreError> {
        let bundle_path = self.object_path(&archive.bundle_sha);
        if bundle_path.exists() {
            return self.verify_existing_object(archive, archive_bytes, verified_at_ms);
        }
        let stage = self
            .paths
            .bundle_staging()
            .join(format!(".bundle-object-{}", uuid::Uuid::new_v4().simple()));
        create_private_directory(&stage)?;
        let result = (|| {
            let compiled_bytes = canonical_bundle_bytes(&archive.compiled_bundle);
            let instructions_git_bundle = decode_git_bundle(
                &archive.instructions_git_bundle_base64,
                "instructions Git bundle",
            )?;
            let personal_git_bundle =
                decode_git_bundle(&archive.personal_git_bundle_base64, "personal Git bundle")?;
            let manifest = object_manifest(
                archive,
                archive_bytes,
                &compiled_bytes,
                &instructions_git_bundle,
                &personal_git_bundle,
            )?;
            write_private_file(&stage.join(MANIFEST_FILE), &canonical_json_bytes(&manifest))?;
            write_private_file(&stage.join(BUNDLE_FILE), &compiled_bytes)?;
            write_private_file(
                &stage.join(INSTRUCTIONS_GIT_BUNDLE_FILE),
                &instructions_git_bundle,
            )?;
            write_private_file(&stage.join(PERSONAL_GIT_BUNDLE_FILE), &personal_git_bundle)?;
            write_private_file(&stage.join(ARCHIVE_FILE), archive_bytes)?;
            sync_directory(&stage)?;
            fs::rename(&stage, &bundle_path).map_err(|_| materialization_error())?;
            sync_directory(self.paths.bundles())?;
            let object = object_record(archive, &manifest, verified_at_ms);
            self.store.save_bundle_object(&object)?;
            Ok(MaterializedBundle {
                object,
                archive_path: bundle_path.join(ARCHIVE_FILE),
            })
        })();
        if stage.exists() {
            let _ = fs::remove_dir_all(&stage);
        }
        result
    }

    fn verify_existing_object(
        &self,
        archive: &BundleArchiveV1,
        archive_bytes: &[u8],
        verified_at_ms: u64,
    ) -> Result<MaterializedBundle, BundleStoreError> {
        let bundle_path = self.object_path(&archive.bundle_sha);
        let metadata = fs::symlink_metadata(&bundle_path)
            .map_err(|_| integrity_error("The bundle object is unavailable"))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(integrity_error("The bundle object path is unsafe"));
        }
        let stored_archive =
            read_private_bounded_file(&bundle_path.join(ARCHIVE_FILE), MAX_BUNDLE_ARCHIVE_BYTES)?;
        if stored_archive != archive_bytes {
            return Err(integrity_error(
                "The existing bundle object does not match its content identity",
            ));
        }
        let manifest_bytes =
            read_private_bounded_file(&bundle_path.join(MANIFEST_FILE), 1024 * 1024)?;
        let manifest: BundleObjectManifestV1 = serde_json::from_slice(&manifest_bytes)
            .map_err(|_| integrity_error("The bundle object manifest is invalid"))?;
        let compiled_bytes =
            read_private_bounded_file(&bundle_path.join(BUNDLE_FILE), 16 * 1024 * 1024)?;
        let instructions_bundle = read_private_bounded_file(
            &bundle_path.join(INSTRUCTIONS_GIT_BUNDLE_FILE),
            MAX_CONFIGURATION_GIT_BUNDLE_BYTES,
        )?;
        let personal_bundle = read_private_bounded_file(
            &bundle_path.join(PERSONAL_GIT_BUNDLE_FILE),
            MAX_CONFIGURATION_GIT_BUNDLE_BYTES,
        )?;
        let expected = object_manifest(
            archive,
            archive_bytes,
            &compiled_bytes,
            &instructions_bundle,
            &personal_bundle,
        )?;
        if manifest != expected
            || compiled_bytes != canonical_bundle_bytes(&archive.compiled_bundle)
        {
            return Err(integrity_error(
                "The existing bundle object failed component verification",
            ));
        }
        let generated = object_record(archive, &manifest, verified_at_ms);
        let object = match self.store.get_bundle_object(&archive.bundle_sha)? {
            Some(existing) => {
                let expected = object_record(archive, &manifest, existing.verified_at_ms);
                if existing != expected {
                    return Err(integrity_error(
                        "The stored bundle metadata conflicts with its verified object",
                    ));
                }
                existing
            }
            None => {
                self.store.save_bundle_object(&generated)?;
                generated
            }
        };
        Ok(MaterializedBundle {
            object,
            archive_path: bundle_path.join(ARCHIVE_FILE),
        })
    }

    fn verify_archive(
        &self,
        archive: &BundleArchiveV1,
        _environment_id: Option<&EnvironmentId>,
    ) -> Result<(), BundleStoreError> {
        if archive.schema_version != BUNDLE_ARCHIVE_SCHEMA_VERSION
            || archive.compiled_bundle.schema_version
                != pandamux_core::COMPILED_BUNDLE_SCHEMA_VERSION
            || archive.bundle_sha.as_str() != archive.compiled_bundle.payload_sha256
            || archive.source_manifest != archive.compiled_bundle.source_manifest
            || archive.compiler_version != archive.compiled_bundle.compiler_version
        {
            return Err(integrity_error(
                "The bundle archive schema or compiled identity is invalid",
            ));
        }
        verify_compiled_bundle(&archive.compiled_bundle)
            .map_err(|_| integrity_error("The compiled bundle payload failed verification"))?;
        let instructions = decode_documents(&archive.instructions_documents)?;
        let project = decode_documents(&archive.project_documents)?;
        let personal = decode_documents(&archive.personal_documents)?;
        validate_documents(&instructions)?;
        validate_documents(&project)?;
        validate_documents(&personal)?;
        match &archive.source_manifest.project_layer_content_sha256 {
            Some(expected) if source_layer_sha256(&project) == *expected => {}
            None if project.is_empty() => {}
            _ => {
                return Err(integrity_error(
                    "The project layer does not match its exact content identity",
                ));
            }
        }
        let instructions_bundle = decode_git_bundle(
            &archive.instructions_git_bundle_base64,
            "instructions Git bundle",
        )?;
        let personal_bundle =
            decode_git_bundle(&archive.personal_git_bundle_base64, "personal Git bundle")?;
        self.verify_git_snapshot(
            &instructions_bundle,
            &archive.instructions_ref,
            &archive.source_manifest.instructions_commit_sha,
            &instructions,
        )?;
        let personal_commit = archive
            .source_manifest
            .personal_commit_sha
            .as_ref()
            .ok_or_else(|| integrity_error("The personal source commit is missing"))?;
        self.verify_git_snapshot(
            &personal_bundle,
            &archive.personal_ref,
            personal_commit,
            &personal,
        )?;
        Ok(())
    }

    fn verify_git_snapshot(
        &self,
        bundle_bytes: &[u8],
        advertised_ref: &str,
        expected_commit: &SourceCommitSha,
        documents: &[SourceDocument],
    ) -> Result<(), BundleStoreError> {
        if !valid_export_ref(advertised_ref) {
            return Err(integrity_error(
                "The archive contains an invalid Git reference",
            ));
        }
        let stage = self
            .paths
            .bundle_staging()
            .join(format!(".verify-git-{}", uuid::Uuid::new_v4().simple()));
        create_private_directory(&stage)?;
        let git_dir = stage.join("repository.git");
        let bundle_path = stage.join("source.bundle");
        let result = (|| {
            self.git.run(ManagedGitCommand::InitBare {
                git_dir: git_dir.clone(),
            })?;
            write_private_file(&bundle_path, bundle_bytes)?;
            let verified = self.git.run(ManagedGitCommand::VerifyBundle {
                git_dir: git_dir.clone(),
                bundle_path: bundle_path.clone(),
            })?;
            let advertised = parse_bundle_heads(&verified.stdout)?;
            if advertised.get(advertised_ref) != Some(expected_commit) || advertised.len() != 1 {
                return Err(integrity_error(
                    "The Git bundle does not advertise the expected source commit",
                ));
            }
            let imported_ref = format!("refs/pandamux/verified/{}", expected_commit.as_str());
            self.git.run(ManagedGitCommand::Fetch {
                git_dir: git_dir.clone(),
                remote: bundle_path.to_string_lossy().into_owned(),
                source: advertised_ref.to_string(),
                target: imported_ref.clone(),
            })?;
            let resolved = self.git.run(ManagedGitCommand::RevParse {
                git_dir: git_dir.clone(),
                revision: imported_ref,
            })?;
            if String::from_utf8(resolved.stdout)
                .ok()
                .map(|value| value.trim().to_string())
                .as_deref()
                != Some(expected_commit.as_str())
            {
                return Err(integrity_error(
                    "The imported Git bundle resolved to a different commit",
                ));
            }
            verify_tree_documents(&self.git, &git_dir, expected_commit, documents)
        })();
        let _ = fs::remove_dir_all(&stage);
        result
    }

    fn incoming_path(&self, transfer_id: &BundleTransferId) -> PathBuf {
        self.paths.bundle_staging().join(format!(
            ".incoming-{}.archive",
            sha256_hex(transfer_id.as_str().as_bytes())
        ))
    }

    fn object_path(&self, bundle_sha: &BundleSha) -> PathBuf {
        self.paths.bundles().join(bundle_sha.as_str())
    }
}

fn archive_from_input(
    input: &BundleMaterializationInput,
) -> Result<BundleArchiveV1, BundleStoreError> {
    verify_compiled_bundle(&input.bundle)
        .map_err(|_| integrity_error("The compiled bundle payload failed verification"))?;
    let bundle_sha = BundleSha::parse(input.bundle.payload_sha256.clone())
        .map_err(|_| integrity_error("The compiled bundle identity is invalid"))?;
    Ok(BundleArchiveV1 {
        schema_version: BUNDLE_ARCHIVE_SCHEMA_VERSION,
        bundle_sha,
        source_manifest: input.bundle.source_manifest.clone(),
        compiler_version: input.bundle.compiler_version.clone(),
        compiled_bundle: input.bundle.clone(),
        instructions_ref: input.instructions_ref.clone(),
        instructions_git_bundle_base64: BASE64.encode(&input.instructions_git_bundle),
        personal_ref: input.personal_ref.clone(),
        personal_git_bundle_base64: BASE64.encode(&input.personal_git_bundle),
        instructions_documents: encode_documents(&input.instructions_documents),
        project_documents: encode_documents(&input.project_documents),
        personal_documents: encode_documents(&input.personal_documents),
    })
}

fn encode_documents(documents: &[SourceDocument]) -> Vec<ArchivedDocument> {
    let mut documents = documents
        .iter()
        .map(|document| ArchivedDocument {
            logical_path: document.logical_path().as_str().to_string(),
            bytes_base64: BASE64.encode(document.bytes()),
        })
        .collect::<Vec<_>>();
    documents.sort_by(|left, right| left.logical_path.cmp(&right.logical_path));
    documents
}

fn decode_documents(
    documents: &[ArchivedDocument],
) -> Result<Vec<SourceDocument>, BundleStoreError> {
    let mut decoded = Vec::with_capacity(documents.len());
    let mut paths = BTreeSet::new();
    for document in documents {
        if !paths.insert(document.logical_path.to_lowercase()) {
            return Err(integrity_error(
                "The bundle archive contains colliding source paths",
            ));
        }
        let bytes = BASE64
            .decode(&document.bytes_base64)
            .map_err(|_| integrity_error("The bundle archive contains invalid source bytes"))?;
        decoded.push(
            SourceDocument::new(document.logical_path.clone(), bytes).map_err(|_| {
                integrity_error("The bundle archive contains an unsafe source path")
            })?,
        );
    }
    Ok(decoded)
}

fn validate_documents(documents: &[SourceDocument]) -> Result<(), BundleStoreError> {
    validate_source_set_bounds(documents)
        .map_err(|_| integrity_error("The archived source set exceeds trusted bounds"))?;
    for document in documents {
        parse_source_document(document)
            .map_err(|_| integrity_error("An archived source document is invalid"))?;
    }
    Ok(())
}

fn decode_git_bundle(encoded: &str, name: &str) -> Result<Vec<u8>, BundleStoreError> {
    let bytes = BASE64
        .decode(encoded)
        .map_err(|_| integrity_error("The archive contains invalid Git bundle bytes"))?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_CONFIGURATION_GIT_BUNDLE_BYTES {
        return Err(integrity_error(&format!("The {name} has an invalid size")));
    }
    Ok(bytes)
}

fn parse_bundle_heads(bytes: &[u8]) -> Result<BTreeMap<String, SourceCommitSha>, BundleStoreError> {
    let output = std::str::from_utf8(bytes)
        .map_err(|_| integrity_error("Git bundle verification returned invalid output"))?;
    let mut heads = BTreeMap::new();
    for line in output.lines() {
        let mut fields = line.split_whitespace();
        let Some(object) = fields.next() else {
            continue;
        };
        let Some(reference) = fields.next() else {
            continue;
        };
        if fields.next().is_some() {
            continue;
        }
        let Ok(object) = SourceCommitSha::parse(object.to_string()) else {
            continue;
        };
        if heads.insert(reference.to_string(), object).is_some() {
            return Err(integrity_error(
                "Git bundle verification returned duplicate references",
            ));
        }
    }
    Ok(heads)
}

fn verify_tree_documents(
    git: &ManagedGitRunner,
    git_dir: &Path,
    commit: &SourceCommitSha,
    documents: &[SourceDocument],
) -> Result<(), BundleStoreError> {
    let output = git.run(ManagedGitCommand::ListTree {
        git_dir: git_dir.to_path_buf(),
        treeish: commit.as_str().to_string(),
    })?;
    let mut tree = BTreeMap::new();
    for record in output.stdout.split(|byte| *byte == 0) {
        if record.is_empty() {
            continue;
        }
        let record = std::str::from_utf8(record)
            .map_err(|_| integrity_error("The verified Git tree contains an invalid path"))?;
        let (metadata, path) = record
            .split_once('\t')
            .ok_or_else(|| integrity_error("The verified Git tree is malformed"))?;
        let mut metadata = metadata.split_whitespace();
        let mode = metadata.next().unwrap_or_default();
        let kind = metadata.next().unwrap_or_default();
        let object = metadata.next().unwrap_or_default();
        if metadata.next().is_some()
            || mode != "100644"
            || kind != "blob"
            || SourceCommitSha::parse(object.to_string()).is_err()
            || tree.insert(path.to_string(), object.to_string()).is_some()
        {
            return Err(integrity_error(
                "The verified Git tree contains an unsupported entry",
            ));
        }
    }
    if tree.len() != documents.len() {
        return Err(integrity_error(
            "The archived documents do not cover the exact verified Git tree",
        ));
    }
    for document in documents {
        let object = tree
            .get(document.logical_path().as_str())
            .ok_or_else(|| integrity_error("An archived document is missing from the Git tree"))?;
        let size = git.run(ManagedGitCommand::CatFileSize {
            git_dir: git_dir.to_path_buf(),
            object: object.clone(),
        })?;
        let size = std::str::from_utf8(&size.stdout)
            .ok()
            .and_then(|value| value.trim().parse::<usize>().ok())
            .ok_or_else(|| integrity_error("A verified Git object has an invalid size"))?;
        if size != document.bytes().len() {
            return Err(integrity_error(
                "An archived document length does not match its Git object",
            ));
        }
        let contents = git.run(ManagedGitCommand::CatFile {
            git_dir: git_dir.to_path_buf(),
            object: object.clone(),
            max_bytes: size,
        })?;
        if contents.stdout != document.bytes() {
            return Err(integrity_error(
                "An archived document does not match its exact Git object",
            ));
        }
    }
    Ok(())
}

fn object_manifest(
    archive: &BundleArchiveV1,
    archive_bytes: &[u8],
    compiled_bytes: &[u8],
    instructions_git_bundle: &[u8],
    personal_git_bundle: &[u8],
) -> Result<BundleObjectManifestV1, BundleStoreError> {
    Ok(BundleObjectManifestV1 {
        schema_version: BUNDLE_ARCHIVE_SCHEMA_VERSION,
        bundle_sha: archive.bundle_sha.clone(),
        source_manifest: archive.source_manifest.clone(),
        compiler_version: archive.compiler_version.clone(),
        compiled_bundle_sha256: content_sha(compiled_bytes)?,
        instructions_git_bundle_sha256: content_sha(instructions_git_bundle)?,
        personal_git_bundle_sha256: content_sha(personal_git_bundle)?,
        archive_sha256: ArchiveSha256::parse(sha256_hex(archive_bytes))
            .map_err(|_| integrity_error("The bundle archive identity is invalid"))?,
        archive_bytes: archive_bytes.len() as u64,
    })
}

fn object_record(
    archive: &BundleArchiveV1,
    manifest: &BundleObjectManifestV1,
    verified_at_ms: u64,
) -> BundleObjectRecord {
    BundleObjectRecord {
        bundle_sha: archive.bundle_sha.clone(),
        source_manifest: archive.source_manifest.clone(),
        payload_sha256: ContentSha256::parse(archive.bundle_sha.as_str().to_string())
            .expect("bundle identity is a content digest"),
        archive_sha256: manifest.archive_sha256.clone(),
        archive_bytes: manifest.archive_bytes,
        compiler_version: archive.compiler_version.clone(),
        object_path: format!("bundles/{}", archive.bundle_sha.as_str()),
        verified_at_ms,
    }
}

fn content_sha(bytes: &[u8]) -> Result<ContentSha256, BundleStoreError> {
    ContentSha256::parse(sha256_hex(bytes))
        .map_err(|_| integrity_error("A bundle component identity is invalid"))
}

fn read_private_bounded_file(path: &Path, max_bytes: u64) -> Result<Vec<u8>, BundleStoreError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| materialization_error())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > max_bytes {
        return Err(integrity_error(
            "A private bundle file is unsafe or oversized",
        ));
    }
    let file = File::open(path).map_err(|_| materialization_error())?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| materialization_error())?;
    if bytes.len() as u64 != metadata.len() || bytes.len() as u64 > max_bytes {
        return Err(integrity_error(
            "A private bundle file changed while it was being read",
        ));
    }
    Ok(bytes)
}

fn create_private_directory(path: &Path) -> Result<(), BundleStoreError> {
    fs::create_dir(path).map_err(|_| materialization_error())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| materialization_error())?;
    }
    Ok(())
}

fn write_private_file(path: &Path, bytes: &[u8]) -> Result<(), BundleStoreError> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|_| materialization_error())?;
    file.write_all(bytes).map_err(|_| materialization_error())?;
    file.sync_all().map_err(|_| materialization_error())
}

fn sync_directory(path: &Path) -> Result<(), BundleStoreError> {
    #[cfg(unix)]
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| materialization_error())?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn valid_export_ref(reference: &str) -> bool {
    reference.starts_with("refs/")
        && !reference.starts_with('-')
        && reference.len() <= 600
        && !reference.bytes().any(|byte| byte.is_ascii_control())
        && !reference.contains("..")
        && !reference.contains([' ', '~', '^', ':', '?', '*', '[', '\\'])
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn materialization_error() -> BundleStoreError {
    BundleStoreError {
        reason: "bundle_materialization_failed",
        safe_message: "The bundle could not be materialized in private storage".to_string(),
        retryable: true,
    }
}

fn integrity_error(message: &str) -> BundleStoreError {
    BundleStoreError {
        reason: "bundle_integrity_failed",
        safe_message: message.to_string(),
        retryable: false,
    }
}

fn state_error() -> BundleStoreError {
    BundleStoreError {
        reason: "bundle_state_failed",
        safe_message: "The bundle state could not be read or persisted".to_string(),
        retryable: true,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use pandamux_core::{
        ConfigurationRevision, ModelTarget, OrganizationId, ProjectId, ProviderInstanceId,
        TierMapping, ValueLayer,
    };
    use pandamux_orchestrator::{
        CompileBundleInput, LayerGrant, LayeredSourceDocument, TrustedBundleSettings,
        compile_instruction_bundle,
    };
    use std::process::Command;

    struct Fixture {
        _temp: tempfile::TempDir,
        store: Store,
        service: BundleStore,
        input: BundleMaterializationInput,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let temp = tempfile::tempdir().expect("temporary bundle fixture");
            let paths = DataPaths::from_root(temp.path().join(name)).expect("paths");
            paths.ensure_private_roots().expect("private roots");
            let store = Store::in_memory().expect("store");
            let service = BundleStore::new(paths.clone(), store.clone());
            let input = materialization_fixture(temp.path());
            Self {
                _temp: temp,
                store,
                service,
                input,
            }
        }
    }

    #[test]
    fn materialization_is_verified_immutable_and_reused() {
        let fixture = Fixture::new("hub");
        let first = fixture
            .service
            .materialize(&fixture.input, 10)
            .expect("materialize bundle");
        let second = fixture
            .service
            .materialize(&fixture.input, 20)
            .expect("reuse bundle");
        assert_eq!(first, second);
        assert_eq!(first.object.verified_at_ms, 10);
        assert!(first.archive_path.is_file());
        assert_eq!(
            fixture
                .store
                .get_bundle_object(&first.object.bundle_sha)
                .expect("load object"),
            Some(first.object.clone())
        );
        let archive = fs::read(&first.archive_path).expect("read archive");
        assert!(
            !archive
                .windows(b"github.com".len())
                .any(|value| value == b"github.com")
        );
        assert!(
            !archive
                .windows(b"credential-sentinel".len())
                .any(|value| { value == b"credential-sentinel" })
        );

        fs::write(&first.archive_path, b"corrupt").expect("corrupt retained object");
        let error = fixture
            .service
            .archive_path(&first.object)
            .expect_err("corrupt object must fail closed");
        assert_eq!(error.reason, "bundle_integrity_failed");
    }

    #[test]
    fn node_rejects_corruption_and_replays_the_original_receipt() {
        let hub = Fixture::new("hub");
        let materialized = hub
            .service
            .materialize(&hub.input, 10)
            .expect("hub materialization");
        let node = Fixture::new("node");
        let environment_id = EnvironmentId::new("node-environment");
        let transfer_id = BundleTransferId::new("bundle-transfer-replay");
        let upload = node
            .service
            .prepare_upload(&transfer_id)
            .expect("prepare node upload");
        fs::copy(&materialized.archive_path, &upload).expect("copy archive");
        let params = BundlePushParams {
            schema_version: BUNDLE_PUSH_SCHEMA_VERSION,
            transfer_id: transfer_id.clone(),
            bundle_sha: materialized.object.bundle_sha.clone(),
            archive_sha256: materialized.object.archive_sha256.clone(),
            archive_bytes: materialized.object.archive_bytes,
            source_manifest: materialized.object.source_manifest.clone(),
            compiler_version: materialized.object.compiler_version.clone(),
            private_artifact_ref: upload.to_string_lossy().into_owned(),
            expected_prior_bundle_sha: None,
        };
        let first = node
            .service
            .receive_uploaded_bundle(&environment_id, &params, 100)
            .expect("receive exact bundle");
        assert_eq!(first.status, BundleDistributionState::Active);
        assert_eq!(first.received_at_ms, 100);

        let replay_upload = node
            .service
            .prepare_upload(&transfer_id)
            .expect("prepare replay");
        fs::copy(&materialized.archive_path, &replay_upload).expect("copy replay archive");
        let replay = node
            .service
            .receive_uploaded_bundle(&environment_id, &params, 200)
            .expect("replay exact transfer");
        assert_eq!(replay, first);
        assert_eq!(
            node.store
                .current_bundle_activation()
                .expect("active node bundle")
                .expect("activation")
                .bundle_sha,
            materialized.object.bundle_sha
        );

        let corrupt_transfer = BundleTransferId::new("bundle-transfer-corrupt");
        let corrupt_upload = node
            .service
            .prepare_upload(&corrupt_transfer)
            .expect("prepare corrupt upload");
        let mut corrupt = fs::read(&materialized.archive_path).expect("read archive");
        corrupt[0] ^= 0x01;
        fs::write(&corrupt_upload, corrupt).expect("write corrupt archive");
        let mut corrupt_params = params;
        corrupt_params.transfer_id = corrupt_transfer;
        corrupt_params.private_artifact_ref = corrupt_upload.to_string_lossy().into_owned();
        let error = node
            .service
            .receive_uploaded_bundle(&environment_id, &corrupt_params, 300)
            .expect_err("corrupt bytes must be rejected");
        assert_eq!(error.reason, "bundle_integrity_failed");

        let bad_source_transfer = BundleTransferId::new("bundle-transfer-bad-source");
        let bad_source_upload = node
            .service
            .prepare_upload(&bad_source_transfer)
            .expect("prepare bad source upload");
        let mut bad_source: BundleArchiveV1 = serde_json::from_slice(
            &fs::read(&materialized.archive_path).expect("read archive for source mutation"),
        )
        .expect("decode source archive");
        let source_bytes = BASE64
            .decode(&bad_source.instructions_documents[0].bytes_base64)
            .expect("decode source document");
        let mut changed_source_bytes = source_bytes;
        changed_source_bytes.extend_from_slice(b"\n");
        bad_source.instructions_documents[0].bytes_base64 = BASE64.encode(changed_source_bytes);
        let bad_source_bytes = canonical_json_bytes(&bad_source);
        fs::write(&bad_source_upload, &bad_source_bytes).expect("write bad source archive");
        let mut bad_source_params = corrupt_params.clone();
        bad_source_params.transfer_id = bad_source_transfer;
        bad_source_params.archive_sha256 =
            ArchiveSha256::parse(sha256_hex(&bad_source_bytes)).expect("bad source archive digest");
        bad_source_params.archive_bytes = bad_source_bytes.len() as u64;
        bad_source_params.private_artifact_ref = bad_source_upload.to_string_lossy().into_owned();
        let error = node
            .service
            .receive_uploaded_bundle(&environment_id, &bad_source_params, 400)
            .expect_err("source bytes outside the Git snapshot must be rejected");
        assert_eq!(error.reason, "bundle_integrity_failed");

        let bad_layer_transfer = BundleTransferId::new("bundle-transfer-bad-layer");
        let bad_layer_upload = node
            .service
            .prepare_upload(&bad_layer_transfer)
            .expect("prepare bad layer upload");
        let mut bad_layer: BundleArchiveV1 = serde_json::from_slice(
            &fs::read(&materialized.archive_path).expect("read archive for layer mutation"),
        )
        .expect("decode layer archive");
        bad_layer.project_documents = encode_documents(&[SourceDocument::new(
            "prompts/untrusted.md",
            prompt("untrusted", "untrusted"),
        )
        .expect("untrusted project document")]);
        let bad_layer_bytes = canonical_json_bytes(&bad_layer);
        fs::write(&bad_layer_upload, &bad_layer_bytes).expect("write bad layer archive");
        let mut bad_layer_params = bad_source_params;
        bad_layer_params.transfer_id = bad_layer_transfer;
        bad_layer_params.archive_sha256 =
            ArchiveSha256::parse(sha256_hex(&bad_layer_bytes)).expect("bad layer archive digest");
        bad_layer_params.archive_bytes = bad_layer_bytes.len() as u64;
        bad_layer_params.private_artifact_ref = bad_layer_upload.to_string_lossy().into_owned();
        let error = node
            .service
            .receive_uploaded_bundle(&environment_id, &bad_layer_params, 500)
            .expect_err("project bytes without an exact layer hash must be rejected");
        assert_eq!(error.reason, "bundle_integrity_failed");
    }

    pub(crate) fn materialization_fixture(root: &Path) -> BundleMaterializationInput {
        let instructions_documents = instructions_documents();
        let instructions_repo = root.join("instructions-source");
        let (instructions_commit, instructions_git_bundle) =
            commit_documents(&instructions_repo, &instructions_documents);
        let personal_repo = root.join("personal-source");
        let (personal_commit, personal_git_bundle) = commit_documents(&personal_repo, &[]);
        let organization_id = OrganizationId::new("organization-bundle-test");
        let project_id = ProjectId::new("project-bundle-test");
        let instructions_hash = source_layer_sha256(&instructions_documents);
        let source_manifest = BundleSourceManifest {
            instructions_commit_sha: instructions_commit,
            project_layer_content_sha256: None,
            personal_commit_sha: Some(personal_commit),
            settings_revision: ConfigurationRevision::new(1).expect("revision"),
        };
        let bundle = compile_instruction_bundle(&CompileBundleInput {
            documents: instructions_documents
                .iter()
                .cloned()
                .map(|source| LayeredSourceDocument {
                    layer: ValueLayer::Instructions,
                    source,
                })
                .collect(),
            grants: vec![LayerGrant {
                layer: ValueLayer::Instructions,
                trusted: true,
                content_sha256: instructions_hash,
                organization_id: organization_id.clone(),
                project_id: None,
            }],
            source_manifest,
            project_id,
            organization_id,
            settings: TrustedBundleSettings {
                revision: ConfigurationRevision::new(1).expect("revision"),
                tier_mapping: TierMapping {
                    tiers: BTreeMap::from([(
                        "reasoning".to_string(),
                        ModelTarget {
                            provider_instance_id: ProviderInstanceId::new("provider-test"),
                            model: "test-model".to_string(),
                            effort: Some("high".to_string()),
                        },
                    )]),
                },
                disabled_agent_ids: BTreeSet::new(),
                full_access_agent_ids: BTreeSet::new(),
            },
        })
        .expect("compile fixture bundle");
        BundleMaterializationInput {
            bundle,
            instructions_documents,
            project_documents: Vec::new(),
            personal_documents: Vec::new(),
            instructions_ref: "refs/heads/main".to_string(),
            instructions_git_bundle,
            personal_ref: "refs/heads/main".to_string(),
            personal_git_bundle,
        }
    }

    fn instructions_documents() -> Vec<SourceDocument> {
        vec![
            SourceDocument::new(
                "orchestrator.yaml",
                include_bytes!(
                    "../../pandamux-orchestrator/tests/fixtures/schema/orchestrator.yaml"
                )
                .to_vec(),
            )
            .expect("orchestrator"),
            SourceDocument::new("agents/builder.md", agent()).expect("agent"),
            SourceDocument::new(
                "knowledge/review-checklist.md",
                include_bytes!("../../pandamux-orchestrator/tests/fixtures/schema/knowledge.md")
                    .to_vec(),
            )
            .expect("knowledge"),
            SourceDocument::new("prompts/intake.md", prompt("intake", "intake")).expect("intake"),
            SourceDocument::new("prompts/planner.md", prompt("planner", "planner"))
                .expect("planner"),
            SourceDocument::new("prompts/handoff.md", prompt("handoff", "handoff"))
                .expect("handoff"),
        ]
    }

    fn prompt(id: &str, purpose: &str) -> String {
        format!(
            "---\nschemaVersion: 1\nkind: pandamux.prompt\nid: {id}\npurpose: {purpose}\n---\nPrompt {id}.\n"
        )
    }

    fn agent() -> String {
        "---\n\
         schemaVersion: 1\n\
         kind: pandamux.agent\n\
         id: builder\n\
         name: Builder\n\
         description: Builds deterministic changes.\n\
         specialties: [implementation]\n\
         tier: reasoning\n\
         effort: high\n\
         accessMode: read_only\n\
         requiredCapabilities: [longContext]\n\
         providers: { prefer: [codex, claude], avoid: [] }\n\
         environments: [local]\n\
         knowledge: [knowledge/review-checklist.md]\n\
         tools: { allow: [read, grep], deny: [write, shell] }\n\
         output: { kind: change_set }\n\
         verification: [cargo test]\n\
         concurrency: 2\n\
         budget: { maxTokens: 400000, maxCostMicroUsd: 5000000, maxRuntimeMs: 1800000 }\n\
         ---\n\
         Build the requested change without widening authority.\n"
            .to_string()
    }

    fn commit_documents(
        repository: &Path,
        documents: &[SourceDocument],
    ) -> (SourceCommitSha, Vec<u8>) {
        fs::create_dir(repository).expect("repository directory");
        git(repository, &["init", "-q"]);
        git(repository, &["symbolic-ref", "HEAD", "refs/heads/main"]);
        for document in documents {
            let path = repository.join(document.logical_path().as_str());
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("document parent");
            }
            fs::write(path, document.bytes()).expect("write document");
        }
        git(repository, &["add", "."]);
        git(
            repository,
            &[
                "-c",
                "user.name=PandaMUX Test",
                "-c",
                "user.email=test@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--allow-empty",
                "-q",
                "-m",
                "fixture",
            ],
        );
        let commit = git(repository, &["rev-parse", "HEAD"]);
        let bundle_path = repository.join("source.bundle");
        git(
            repository,
            &[
                "bundle",
                "create",
                bundle_path.to_str().expect("bundle path"),
                "refs/heads/main",
            ],
        );
        (
            SourceCommitSha::parse(commit.trim().to_string()).expect("commit identity"),
            fs::read(bundle_path).expect("read Git bundle"),
        )
    }

    fn git(repository: &Path, arguments: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(repository)
            .args(arguments)
            .output()
            .expect("run Git fixture command");
        assert!(
            output.status.success(),
            "Git fixture failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("Git fixture output")
    }
}
