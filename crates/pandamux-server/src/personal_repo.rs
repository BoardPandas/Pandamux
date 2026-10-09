use crate::managed_git::{
    GitCommitIdentity, GitTreeEntry, ManagedGitCommand, ManagedGitError, ManagedGitRunner,
};
use crate::{DataPaths, PersonalRepoStateRecord, Store, StoreError};
use pandamux_core::{ConfigurationRevision, ContentSha256, RunId, SecretsPolicy, SourceCommitSha};
use pandamux_orchestrator::{
    SourceDocument, canonical_json_sha256, parse_source_document, validate_source_set_bounds,
};
use pandamux_protocol::PersonalConfigHistoryEntry;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};

const PERSONAL_MAIN_REF: &str = "refs/heads/main";
const PERSONAL_REQUEST_REF_PREFIX: &str = "refs/pandamux/requests/";
const INIT_CANDIDATE_PREFIX: &str = ".personal-init-";
const PROJECTION_CANDIDATE_PREFIX: &str = ".personal-projection-";
const MAX_REQUEST_ID_BYTES: usize = 256;
const MAX_CALLER_BYTES: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PersonalRepoFailpoint {
    AfterDirectory,
    AfterObjects,
    AfterCommit,
    BeforeRef,
    AfterRef,
    BeforeReconciliation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PersonalEditRequest {
    pub request_id: String,
    pub caller: String,
    pub accepted_at_ms: u64,
    pub originating_run_id: Option<RunId>,
    pub expected_head: SourceCommitSha,
    pub files: BTreeMap<String, Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonalEditResult {
    pub source_commit_sha: SourceCommitSha,
    pub parent_source_commit_sha: SourceCommitSha,
    pub change_digest: ContentSha256,
    pub revision: ConfigurationRevision,
    pub duplicate: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PersonalConfigDiff {
    pub added: Vec<String>,
    pub modified: Vec<String>,
    pub deleted: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PersonalRepoError {
    pub reason: &'static str,
    pub safe_message: String,
    pub retryable: bool,
}

impl std::fmt::Display for PersonalRepoError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.safe_message)
    }
}

impl std::error::Error for PersonalRepoError {}

impl From<ManagedGitError> for PersonalRepoError {
    fn from(error: ManagedGitError) -> Self {
        Self {
            reason: error.reason,
            safe_message: error.safe_message,
            retryable: error.retryable,
        }
    }
}

impl From<StoreError> for PersonalRepoError {
    fn from(_: StoreError) -> Self {
        Self {
            reason: "personal_state_failed",
            safe_message: "The personal repository state could not be persisted".to_string(),
            retryable: true,
        }
    }
}

#[derive(Clone)]
pub struct PersonalRepoService {
    paths: DataPaths,
    git: ManagedGitRunner,
    store: Store,
    owner: Arc<Mutex<()>>,
}

impl PersonalRepoService {
    pub fn new(paths: DataPaths, store: Store) -> Self {
        let owner = repository_owner(paths.personal());
        Self {
            git: ManagedGitRunner::new(paths.clone()),
            paths,
            store,
            owner,
        }
    }

    pub fn shutdown(&self) {
        self.git.shutdown();
    }

    pub fn ensure_initialized(&self) -> Result<SourceCommitSha, PersonalRepoError> {
        self.ensure_initialized_with_failpoint(None)
    }

    pub fn ensure_initialized_with_failpoint(
        &self,
        failpoint: Option<PersonalRepoFailpoint>,
    ) -> Result<SourceCommitSha, PersonalRepoError> {
        let _owner = self.owner.lock().expect("personal repository owner lock");
        self.ensure_initialized_owned(failpoint)
    }

    pub fn apply_edit(
        &self,
        request: &PersonalEditRequest,
    ) -> Result<PersonalEditResult, PersonalRepoError> {
        self.apply_edit_with_failpoint(request, None)
    }

    pub fn apply_edit_with_failpoint(
        &self,
        request: &PersonalEditRequest,
        failpoint: Option<PersonalRepoFailpoint>,
    ) -> Result<PersonalEditResult, PersonalRepoError> {
        validate_request_metadata(request)?;
        validate_snapshot(&request.files)?;
        let request_digest = snapshot_digest(request);
        let request_identity = request_identity(&request.caller, &request.request_id);

        let _owner = self.owner.lock().expect("personal repository owner lock");
        self.ensure_initialized_owned(None)?;
        let git_dir = self.git_dir();
        if let Some(existing) = self.request_commit(&git_dir, &request_identity)? {
            return self.replay_result(&git_dir, &existing, &request_digest);
        }

        let current_head = self.current_head(&git_dir)?;
        if current_head != request.expected_head {
            return Err(PersonalRepoError {
                reason: "personal_head_changed",
                safe_message: "The personal configuration changed before the edit was accepted"
                    .to_string(),
                retryable: true,
            });
        }
        let revision = self.next_revision(&git_dir, &current_head)?;
        let tree = self.write_snapshot_tree(&git_dir, &request.files)?;
        fail_if(failpoint, PersonalRepoFailpoint::AfterObjects)?;
        let message = edit_commit_message(request, &request_identity, &request_digest, revision);
        let commit = self.create_commit(
            &git_dir,
            &tree,
            Some(&current_head),
            &message,
            request.accepted_at_ms,
        )?;
        fail_if(failpoint, PersonalRepoFailpoint::AfterCommit)?;
        fail_if(failpoint, PersonalRepoFailpoint::BeforeRef)?;
        let request_ref = request_ref(&request_identity);
        let transaction = format!(
            "start\ncreate {request_ref} {commit}\nupdate {PERSONAL_MAIN_REF} {commit} {current_head}\nprepare\ncommit\n"
        );
        if let Err(error) = self.git.run(ManagedGitCommand::UpdateRefs {
            git_dir: git_dir.clone(),
            transaction,
        }) {
            if let Some(existing) = self.request_commit(&git_dir, &request_identity)? {
                return self.replay_result(&git_dir, &existing, &request_digest);
            }
            return Err(PersonalRepoError {
                reason: "personal_head_changed",
                safe_message: "The personal configuration changed before the edit was accepted"
                    .to_string(),
                retryable: true,
            }
            .with_git_context(error));
        }
        fail_if(failpoint, PersonalRepoFailpoint::AfterRef)?;
        fail_if(failpoint, PersonalRepoFailpoint::BeforeReconciliation)?;

        let result = PersonalEditResult {
            source_commit_sha: commit.clone(),
            parent_source_commit_sha: current_head,
            change_digest: request_digest.clone(),
            revision,
            duplicate: false,
        };
        self.reconcile_after_ref(
            &git_dir,
            &PersonalRepoStateRecord {
                head_source_commit_sha: commit,
                revision,
                last_request_identity: Some(request_identity),
                last_request_digest: Some(request_digest),
                last_result: Some(
                    serde_json::to_value(&result).map_err(|_| personal_state_error())?,
                ),
                updated_at_ms: request.accepted_at_ms,
            },
        )?;
        Ok(result)
    }

    pub fn revert(
        &self,
        request_id: String,
        caller: String,
        accepted_at_ms: u64,
        originating_run_id: Option<RunId>,
        expected_head: SourceCommitSha,
        target: SourceCommitSha,
    ) -> Result<PersonalEditResult, PersonalRepoError> {
        let _owner = self.owner.lock().expect("personal repository owner lock");
        self.ensure_initialized_owned(None)?;
        let snapshot = self.read_snapshot(&self.git_dir(), target.as_str())?;
        drop(_owner);
        self.apply_edit(&PersonalEditRequest {
            request_id,
            caller,
            accepted_at_ms,
            originating_run_id,
            expected_head,
            files: snapshot,
        })
    }

    pub fn history(
        &self,
        limit: usize,
    ) -> Result<Vec<PersonalConfigHistoryEntry>, PersonalRepoError> {
        let _owner = self.owner.lock().expect("personal repository owner lock");
        self.ensure_initialized_owned(None)?;
        let git_dir = self.git_dir();
        let output = self.git.run(ManagedGitCommand::RevList {
            git_dir: git_dir.clone(),
            revision: PERSONAL_MAIN_REF.to_string(),
            max_count: limit.clamp(1, 1000),
        })?;
        let text = parse_utf8(&output.stdout)?;
        let mut entries = Vec::new();
        for commit in text.lines().filter(|line| !line.trim().is_empty()) {
            let commit = SourceCommitSha::parse(commit.trim()).map_err(|_| corrupt_repository())?;
            let message = self.commit_message(&git_dir, &commit)?;
            if message.starts_with("Initialize PandaMUX personal configuration") {
                continue;
            }
            let trailers = parse_trailers(&message);
            let request_id = trailers
                .get("PandaMUX-Request-ID")
                .cloned()
                .ok_or_else(corrupt_repository)?;
            let caller = trailers
                .get("PandaMUX-Caller")
                .cloned()
                .ok_or_else(corrupt_repository)?;
            let accepted_at_ms = trailers
                .get("PandaMUX-Accepted-At-Ms")
                .and_then(|value| value.parse::<u64>().ok())
                .ok_or_else(corrupt_repository)?;
            let parent = self.optional_parent(&git_dir, &commit)?;
            entries.push(PersonalConfigHistoryEntry {
                source_commit_sha: commit,
                parent_source_commit_sha: parent,
                request_id,
                caller,
                accepted_at_ms,
            });
        }
        Ok(entries)
    }

    pub fn diff(
        &self,
        from: &SourceCommitSha,
        to: &SourceCommitSha,
    ) -> Result<PersonalConfigDiff, PersonalRepoError> {
        let _owner = self.owner.lock().expect("personal repository owner lock");
        self.ensure_initialized_owned(None)?;
        let git_dir = self.git_dir();
        let from = self.read_snapshot(&git_dir, from.as_str())?;
        let to = self.read_snapshot(&git_dir, to.as_str())?;
        let mut added = Vec::new();
        let mut modified = Vec::new();
        let mut deleted = Vec::new();
        for (path, bytes) in &to {
            match from.get(path) {
                None => added.push(path.clone()),
                Some(previous) if previous != bytes => modified.push(path.clone()),
                Some(_) => {}
            }
        }
        for path in from.keys() {
            if !to.contains_key(path) {
                deleted.push(path.clone());
            }
        }
        Ok(PersonalConfigDiff {
            added,
            modified,
            deleted,
        })
    }

    pub fn current_head_sha(&self) -> Result<SourceCommitSha, PersonalRepoError> {
        let _owner = self.owner.lock().expect("personal repository owner lock");
        self.ensure_initialized_owned(None)
    }

    fn ensure_initialized_owned(
        &self,
        failpoint: Option<PersonalRepoFailpoint>,
    ) -> Result<SourceCommitSha, PersonalRepoError> {
        self.paths
            .ensure_private_roots()
            .map_err(|_| managed_path_error())?;
        self.recover_init_candidates()?;
        self.recover_projection_candidates()?;
        if self.paths.personal().exists() {
            if let Ok(head) = self.current_head(&self.git_dir()) {
                let revision = self.revision_for_head(&self.git_dir(), &head)?;
                self.reconcile_after_ref(
                    &self.git_dir(),
                    &PersonalRepoStateRecord {
                        head_source_commit_sha: head.clone(),
                        revision,
                        last_request_identity: None,
                        last_request_digest: None,
                        last_result: None,
                        updated_at_ms: now_ms(),
                    },
                )?;
                return Ok(head);
            }
            if directory_is_empty(self.paths.personal())? {
                fs::remove_dir(self.paths.personal()).map_err(|_| managed_path_error())?;
            } else {
                return Err(PersonalRepoError {
                    reason: "personal_repo_recovering",
                    safe_message: "The personal repository requires recovery before use"
                        .to_string(),
                    retryable: true,
                });
            }
        }

        let candidate = self.paths.root().join(format!(
            "{INIT_CANDIDATE_PREFIX}{}",
            uuid::Uuid::new_v4().simple()
        ));
        create_private_directory(&candidate)?;
        fail_if(failpoint, PersonalRepoFailpoint::AfterDirectory)?;
        let git_dir = candidate.join(".git");
        self.git.run(ManagedGitCommand::InitBare {
            git_dir: git_dir.clone(),
        })?;
        self.git.run(ManagedGitCommand::SymbolicRef {
            git_dir: git_dir.clone(),
            name: "HEAD".to_string(),
            target: PERSONAL_MAIN_REF.to_string(),
        })?;
        let empty_tree = self.write_tree(&git_dir, Vec::new())?;
        fail_if(failpoint, PersonalRepoFailpoint::AfterObjects)?;
        let commit = self.create_commit(
            &git_dir,
            &empty_tree,
            None,
            "Initialize PandaMUX personal configuration\n",
            1_000,
        )?;
        fail_if(failpoint, PersonalRepoFailpoint::AfterCommit)?;
        fail_if(failpoint, PersonalRepoFailpoint::BeforeRef)?;
        self.git.run(ManagedGitCommand::UpdateRefs {
            git_dir: git_dir.clone(),
            transaction: format!("start\ncreate {PERSONAL_MAIN_REF} {commit}\nprepare\ncommit\n"),
        })?;
        fail_if(failpoint, PersonalRepoFailpoint::AfterRef)?;
        self.current_head(&git_dir)?;
        sync_tree(&candidate)?;
        install_candidate(&candidate, self.paths.personal(), self.paths.recovery())?;
        fail_if(failpoint, PersonalRepoFailpoint::BeforeReconciliation)?;
        let installed_head = self.current_head(&self.git_dir())?;
        let installed_revision = self.revision_for_head(&self.git_dir(), &installed_head)?;
        self.reconcile_after_ref(
            &self.git_dir(),
            &PersonalRepoStateRecord {
                head_source_commit_sha: installed_head.clone(),
                revision: installed_revision,
                last_request_identity: None,
                last_request_digest: None,
                last_result: None,
                updated_at_ms: now_ms(),
            },
        )?;
        Ok(installed_head)
    }

    fn recover_init_candidates(&self) -> Result<(), PersonalRepoError> {
        let entries = fs::read_dir(self.paths.root()).map_err(|_| managed_path_error())?;
        for entry in entries {
            let entry = entry.map_err(|_| managed_path_error())?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with(INIT_CANDIDATE_PREFIX) {
                continue;
            }
            let candidate = entry.path();
            let git_dir = candidate.join(".git");
            let valid_head = self.current_head(&git_dir).ok();
            if !self.paths.personal().exists() && valid_head.is_some() {
                match fs::rename(&candidate, self.paths.personal()) {
                    Ok(()) => sync_directory(self.paths.root())?,
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                        move_owned_candidate_to_recovery(&candidate, self.paths.recovery())?;
                    }
                    Err(_) => return Err(managed_path_error()),
                }
            } else {
                move_owned_candidate_to_recovery(&candidate, self.paths.recovery())?;
            }
        }
        Ok(())
    }

    fn recover_projection_candidates(&self) -> Result<(), PersonalRepoError> {
        let entries = fs::read_dir(self.paths.root()).map_err(|_| managed_path_error())?;
        for entry in entries {
            let entry = entry.map_err(|_| managed_path_error())?;
            let name = entry.file_name();
            if name
                .to_str()
                .is_some_and(|name| name.starts_with(PROJECTION_CANDIDATE_PREFIX))
            {
                move_owned_candidate_to_recovery(&entry.path(), self.paths.recovery())?;
            }
        }
        Ok(())
    }

    fn replay_result(
        &self,
        git_dir: &Path,
        commit: &SourceCommitSha,
        expected_digest: &ContentSha256,
    ) -> Result<PersonalEditResult, PersonalRepoError> {
        let message = self.commit_message(git_dir, commit)?;
        let trailers = parse_trailers(&message);
        let recorded_digest = trailers
            .get("PandaMUX-Request-Digest")
            .ok_or_else(corrupt_repository)
            .and_then(|value| {
                ContentSha256::parse(value.clone()).map_err(|_| corrupt_repository())
            })?;
        if &recorded_digest != expected_digest {
            return Err(PersonalRepoError {
                reason: "request_identity_conflict",
                safe_message: "The personal edit request identity was reused with different bytes"
                    .to_string(),
                retryable: false,
            });
        }
        let revision = trailers
            .get("PandaMUX-Revision")
            .and_then(|value| value.parse::<u64>().ok())
            .and_then(|value| ConfigurationRevision::new(value).ok())
            .ok_or_else(corrupt_repository)?;
        let parent = self
            .optional_parent(git_dir, commit)?
            .ok_or_else(corrupt_repository)?;
        let head = self.current_head(git_dir)?;
        self.reconcile_after_ref(
            git_dir,
            &PersonalRepoStateRecord {
                head_source_commit_sha: head.clone(),
                revision: self.revision_for_head(git_dir, &head)?,
                last_request_identity: None,
                last_request_digest: None,
                last_result: None,
                updated_at_ms: now_ms(),
            },
        )?;
        Ok(PersonalEditResult {
            source_commit_sha: commit.clone(),
            parent_source_commit_sha: parent,
            change_digest: recorded_digest,
            revision,
            duplicate: true,
        })
    }

    fn request_commit(
        &self,
        git_dir: &Path,
        identity: &ContentSha256,
    ) -> Result<Option<SourceCommitSha>, PersonalRepoError> {
        match self.git.run(ManagedGitCommand::RevParse {
            git_dir: git_dir.to_path_buf(),
            revision: request_ref(identity),
        }) {
            Ok(output) => SourceCommitSha::parse(parse_trimmed(&output.stdout)?)
                .map(Some)
                .map_err(|_| corrupt_repository()),
            Err(error) if error.reason == "git_command_failed" => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn current_head(&self, git_dir: &Path) -> Result<SourceCommitSha, PersonalRepoError> {
        let output = self.git.run(ManagedGitCommand::RevParse {
            git_dir: git_dir.to_path_buf(),
            revision: PERSONAL_MAIN_REF.to_string(),
        })?;
        SourceCommitSha::parse(parse_trimmed(&output.stdout)?).map_err(|_| corrupt_repository())
    }

    fn optional_parent(
        &self,
        git_dir: &Path,
        commit: &SourceCommitSha,
    ) -> Result<Option<SourceCommitSha>, PersonalRepoError> {
        match self.git.run(ManagedGitCommand::RevParse {
            git_dir: git_dir.to_path_buf(),
            revision: format!("{}^", commit.as_str()),
        }) {
            Ok(output) => SourceCommitSha::parse(parse_trimmed(&output.stdout)?)
                .map(Some)
                .map_err(|_| corrupt_repository()),
            Err(error) if error.reason == "git_command_failed" => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn commit_message(
        &self,
        git_dir: &Path,
        commit: &SourceCommitSha,
    ) -> Result<String, PersonalRepoError> {
        let output = self.git.run(ManagedGitCommand::ShowMessage {
            git_dir: git_dir.to_path_buf(),
            commit: commit.as_str().to_string(),
        })?;
        parse_utf8(&output.stdout)
    }

    fn next_revision(
        &self,
        git_dir: &Path,
        head: &SourceCommitSha,
    ) -> Result<ConfigurationRevision, PersonalRepoError> {
        self.revision_for_head(git_dir, head)?
            .checked_next()
            .map_err(|_| personal_state_error())
    }

    fn revision_for_head(
        &self,
        git_dir: &Path,
        head: &SourceCommitSha,
    ) -> Result<ConfigurationRevision, PersonalRepoError> {
        if let Some(state) = self.store.get_personal_repo_state()?
            && state.head_source_commit_sha == *head
        {
            return Ok(state.revision);
        }
        let message = self.commit_message(git_dir, head)?;
        if message.starts_with("Initialize PandaMUX personal configuration") {
            return Ok(ConfigurationRevision::INITIAL);
        }
        parse_trailers(&message)
            .get("PandaMUX-Revision")
            .and_then(|value| value.parse::<u64>().ok())
            .and_then(|value| ConfigurationRevision::new(value).ok())
            .ok_or_else(corrupt_repository)
    }

    fn create_commit(
        &self,
        git_dir: &Path,
        tree: &SourceCommitSha,
        parent: Option<&SourceCommitSha>,
        message: &str,
        accepted_at_ms: u64,
    ) -> Result<SourceCommitSha, PersonalRepoError> {
        let output = self.git.run(ManagedGitCommand::CommitTree {
            git_dir: git_dir.to_path_buf(),
            tree: tree.as_str().to_string(),
            parent: parent.map(|value| value.as_str().to_string()),
            message: message.to_string(),
            identity: GitCommitIdentity {
                name: "PandaMUX Configuration".to_string(),
                email: "configuration@pandamux.local".to_string(),
                timestamp_seconds: accepted_at_ms / 1000,
            },
        })?;
        SourceCommitSha::parse(parse_trimmed(&output.stdout)?).map_err(|_| corrupt_repository())
    }

    fn write_snapshot_tree(
        &self,
        git_dir: &Path,
        files: &BTreeMap<String, Vec<u8>>,
    ) -> Result<SourceCommitSha, PersonalRepoError> {
        let mut root = TreeNode::default();
        for (path, bytes) in files {
            let output = self.git.run(ManagedGitCommand::HashObject {
                git_dir: git_dir.to_path_buf(),
                bytes: bytes.clone(),
            })?;
            let object = SourceCommitSha::parse(parse_trimmed(&output.stdout)?)
                .map_err(|_| corrupt_repository())?;
            root.insert(path, object)?;
        }
        self.write_tree_node(git_dir, &root)
    }

    fn write_tree_node(
        &self,
        git_dir: &Path,
        node: &TreeNode,
    ) -> Result<SourceCommitSha, PersonalRepoError> {
        let mut entries = Vec::new();
        for (name, child) in &node.children {
            match child {
                TreeChild::Blob(object) => entries.push(GitTreeEntry {
                    mode: "100644",
                    object_kind: "blob",
                    object_id: object.as_str().to_string(),
                    name: name.clone(),
                }),
                TreeChild::Tree(tree) => {
                    let object = self.write_tree_node(git_dir, tree)?;
                    entries.push(GitTreeEntry {
                        mode: "040000",
                        object_kind: "tree",
                        object_id: object.as_str().to_string(),
                        name: name.clone(),
                    });
                }
            }
        }
        self.write_tree(git_dir, entries)
    }

    fn write_tree(
        &self,
        git_dir: &Path,
        entries: Vec<GitTreeEntry>,
    ) -> Result<SourceCommitSha, PersonalRepoError> {
        let output = self.git.run(ManagedGitCommand::MakeTree {
            git_dir: git_dir.to_path_buf(),
            entries,
        })?;
        SourceCommitSha::parse(parse_trimmed(&output.stdout)?).map_err(|_| corrupt_repository())
    }

    fn read_snapshot(
        &self,
        git_dir: &Path,
        treeish: &str,
    ) -> Result<BTreeMap<String, Vec<u8>>, PersonalRepoError> {
        let output = self.git.run(ManagedGitCommand::ListTree {
            git_dir: git_dir.to_path_buf(),
            treeish: treeish.to_string(),
        })?;
        let mut files = BTreeMap::new();
        let mut casefolded = BTreeSet::new();
        for record in output.stdout.split(|byte| *byte == 0) {
            if record.is_empty() {
                continue;
            }
            let record = std::str::from_utf8(record).map_err(|_| corrupt_repository())?;
            let (metadata, path) = record.split_once('\t').ok_or_else(corrupt_repository)?;
            let mut metadata = metadata.split_whitespace();
            let mode = metadata.next().ok_or_else(corrupt_repository)?;
            let kind = metadata.next().ok_or_else(corrupt_repository)?;
            let object = metadata.next().ok_or_else(corrupt_repository)?;
            if mode != "100644" || kind != "blob" || metadata.next().is_some() {
                return Err(corrupt_repository());
            }
            validate_personal_path(path)?;
            if !casefolded.insert(path.to_ascii_lowercase()) {
                return Err(corrupt_repository());
            }
            let blob = self.git.run(ManagedGitCommand::CatFile {
                git_dir: git_dir.to_path_buf(),
                object: object.to_string(),
                max_bytes: pandamux_orchestrator::schema::MAX_MARKDOWN_FILE_BYTES,
            })?;
            files.insert(path.to_string(), blob.stdout);
        }
        validate_snapshot(&files)?;
        Ok(files)
    }

    fn reconcile_projection(
        &self,
        git_dir: &Path,
        head: &SourceCommitSha,
    ) -> Result<(), PersonalRepoError> {
        let desired = self.read_snapshot(git_dir, head.as_str())?;
        let candidate = self.paths.root().join(format!(
            "{PROJECTION_CANDIDATE_PREFIX}{}",
            uuid::Uuid::new_v4().simple()
        ));
        create_private_directory(&candidate)?;
        for (path, bytes) in &desired {
            let destination = candidate.join(path);
            let parent = destination.parent().ok_or_else(managed_path_error)?;
            create_private_directory(parent)?;
            write_private_file(&destination, bytes)?;
        }
        let current = projected_files(self.paths.personal())?;
        for path in current.keys() {
            if !desired.contains_key(path) {
                let target = self.paths.personal().join(path);
                if fs::symlink_metadata(&target)
                    .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
                {
                    fs::remove_file(&target).map_err(|_| managed_path_error())?;
                    sync_directory(target.parent().ok_or_else(managed_path_error)?)?;
                }
            }
        }
        for path in desired.keys() {
            let source = candidate.join(path);
            let destination = self.paths.personal().join(path);
            let parent = destination.parent().ok_or_else(managed_path_error)?;
            create_private_directory(parent)?;
            if destination.exists() {
                let metadata =
                    fs::symlink_metadata(&destination).map_err(|_| managed_path_error())?;
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err(managed_path_error());
                }
                fs::remove_file(&destination).map_err(|_| managed_path_error())?;
            }
            fs::rename(&source, &destination).map_err(|_| managed_path_error())?;
            sync_directory(parent)?;
        }
        remove_owned_projection_candidate(&candidate)?;
        sync_directory(self.paths.personal())?;
        Ok(())
    }

    fn reconcile_after_ref(
        &self,
        git_dir: &Path,
        state: &PersonalRepoStateRecord,
    ) -> Result<(), PersonalRepoError> {
        self.reconcile_projection(git_dir, &state.head_source_commit_sha)?;
        self.store.reconcile_personal_repo_state(state)?;
        Ok(())
    }

    fn git_dir(&self) -> PathBuf {
        self.paths.personal().join(".git")
    }
}

fn repository_owner(path: &Path) -> Arc<Mutex<()>> {
    static OWNERS: OnceLock<Mutex<BTreeMap<PathBuf, Weak<Mutex<()>>>>> = OnceLock::new();
    let owners = OWNERS.get_or_init(|| Mutex::new(BTreeMap::new()));
    let mut owners = owners.lock().expect("personal repository owner registry");
    owners.retain(|_, owner| owner.strong_count() > 0);
    if let Some(owner) = owners.get(path).and_then(Weak::upgrade) {
        return owner;
    }
    let owner = Arc::new(Mutex::new(()));
    owners.insert(path.to_path_buf(), Arc::downgrade(&owner));
    owner
}

impl PersonalRepoError {
    fn with_git_context(self, error: ManagedGitError) -> Self {
        if error.reason == "git_command_failed" {
            self
        } else {
            error.into()
        }
    }
}

#[derive(Default)]
struct TreeNode {
    children: BTreeMap<String, TreeChild>,
}

enum TreeChild {
    Blob(SourceCommitSha),
    Tree(TreeNode),
}

impl TreeNode {
    fn insert(&mut self, path: &str, object: SourceCommitSha) -> Result<(), PersonalRepoError> {
        let mut segments = path.split('/').peekable();
        let mut node = self;
        while let Some(segment) = segments.next() {
            if segments.peek().is_none() {
                if node
                    .children
                    .insert(segment.to_string(), TreeChild::Blob(object.clone()))
                    .is_some()
                {
                    return Err(invalid_configuration());
                }
                return Ok(());
            }
            let child = node
                .children
                .entry(segment.to_string())
                .or_insert_with(|| TreeChild::Tree(TreeNode::default()));
            let TreeChild::Tree(tree) = child else {
                return Err(invalid_configuration());
            };
            node = tree;
        }
        Err(invalid_configuration())
    }
}

fn validate_request_metadata(request: &PersonalEditRequest) -> Result<(), PersonalRepoError> {
    let invalid_run = request.originating_run_id.as_ref().is_some_and(|run_id| {
        run_id.as_str().trim().is_empty()
            || run_id.as_str().len() > MAX_REQUEST_ID_BYTES
            || run_id.as_str().contains(['\0', '\n', '\r'])
            || SecretsPolicy::contains_potential_secret(run_id.as_str())
    });
    if request.request_id.trim().is_empty()
        || request.request_id.len() > MAX_REQUEST_ID_BYTES
        || request.caller.trim().is_empty()
        || request.caller.len() > MAX_CALLER_BYTES
        || request.request_id.contains(['\0', '\n', '\r'])
        || request.caller.contains(['\0', '\n', '\r'])
        || SecretsPolicy::contains_potential_secret(&request.request_id)
        || SecretsPolicy::contains_potential_secret(&request.caller)
        || invalid_run
    {
        return Err(PersonalRepoError {
            reason: "secret_or_runtime_data_rejected",
            safe_message: "The personal edit metadata is invalid or sensitive".to_string(),
            retryable: false,
        });
    }
    Ok(())
}

fn validate_snapshot(
    files: &BTreeMap<String, Vec<u8>>,
) -> Result<Vec<SourceDocument>, PersonalRepoError> {
    if files.len() > pandamux_orchestrator::schema::MAX_SOURCE_FILES
        || files
            .values()
            .try_fold(0usize, |total, bytes| total.checked_add(bytes.len()))
            .is_none_or(|total| total > pandamux_orchestrator::schema::MAX_SOURCE_BYTES)
        || files
            .values()
            .any(|bytes| bytes.len() > pandamux_orchestrator::schema::MAX_MARKDOWN_FILE_BYTES)
    {
        return Err(invalid_configuration());
    }
    let mut documents = Vec::with_capacity(files.len());
    let mut casefolded = BTreeSet::new();
    for path in files.keys() {
        validate_personal_path(path)?;
        if !casefolded.insert(path.to_lowercase()) {
            return Err(PersonalRepoError {
                reason: "case_colliding_configuration_path",
                safe_message: "Personal configuration paths collide by case".to_string(),
                retryable: false,
            });
        }
    }
    for (path, bytes) in files {
        let text = std::str::from_utf8(bytes).map_err(|_| invalid_configuration())?;
        if SecretsPolicy::contains_potential_secret(path)
            || SecretsPolicy::contains_potential_secret(text)
        {
            return Err(PersonalRepoError {
                reason: "secret_or_runtime_data_rejected",
                safe_message: "Sensitive values cannot be stored in personal configuration"
                    .to_string(),
                retryable: false,
            });
        }
        let source = SourceDocument::new(path.clone(), bytes.clone())
            .map_err(|_| invalid_configuration())?;
        documents.push(source);
    }
    validate_source_set_bounds(&documents).map_err(|_| invalid_configuration())?;
    for source in &documents {
        parse_source_document(source).map_err(|_| invalid_configuration())?;
    }
    Ok(documents)
}

fn validate_personal_path(path: &str) -> Result<(), PersonalRepoError> {
    let allowed = path.starts_with("agents/")
        || path.starts_with("knowledge/")
        || path.starts_with("prompts/");
    let runtime_segment = path.split('/').any(|segment| {
        matches!(
            segment.to_ascii_lowercase().as_str(),
            ".git"
                | ".pandamux"
                | "run"
                | "runtime"
                | "data"
                | "attachments"
                | "bundles"
                | "recovery"
                | "target"
        )
    });
    if !allowed || runtime_segment {
        return Err(PersonalRepoError {
            reason: "secret_or_runtime_data_rejected",
            safe_message: "Runtime or unsupported paths cannot be stored in personal configuration"
                .to_string(),
            retryable: false,
        });
    }
    if path.split('/').any(portable_path_segment_is_unsafe) {
        return Err(invalid_configuration());
    }
    SourceDocument::new(path.to_string(), Vec::new()).map_err(|_| invalid_configuration())?;
    Ok(())
}

fn portable_path_segment_is_unsafe(segment: &str) -> bool {
    if segment.is_empty()
        || segment.ends_with(' ')
        || segment.ends_with('.')
        || segment.chars().any(|character| {
            character.is_control() || matches!(character, '<' | '>' | ':' | '"' | '|' | '?' | '*')
        })
    {
        return true;
    }
    let device_name = segment
        .split_once('.')
        .map_or(segment, |(prefix, _)| prefix)
        .to_ascii_uppercase();
    matches!(device_name.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || device_name
            .strip_prefix("COM")
            .or_else(|| device_name.strip_prefix("LPT"))
            .is_some_and(|suffix| matches!(suffix.as_bytes(), [b'1'..=b'9']))
}

fn snapshot_digest(request: &PersonalEditRequest) -> ContentSha256 {
    let mut hasher = Sha256::new();
    hash_part(&mut hasher, request.caller.as_bytes());
    hash_part(&mut hasher, request.request_id.as_bytes());
    match &request.originating_run_id {
        Some(run_id) => {
            hash_part(&mut hasher, b"run");
            hash_part(&mut hasher, run_id.as_str().as_bytes());
        }
        None => hash_part(&mut hasher, b"no-run"),
    }
    for (path, bytes) in &request.files {
        hash_part(&mut hasher, path.as_bytes());
        hash_part(&mut hasher, bytes);
    }
    ContentSha256::parse(hex_digest(hasher.finalize().as_slice())).expect("SHA-256 output is valid")
}

fn request_identity(caller: &str, request_id: &str) -> ContentSha256 {
    let value = serde_json::json!({
        "caller": caller,
        "requestId": request_id,
        "tool": "personalConfig.edit"
    });
    ContentSha256::parse(canonical_json_sha256(&value)).expect("SHA-256 output is valid")
}

fn request_ref(identity: &ContentSha256) -> String {
    format!("{PERSONAL_REQUEST_REF_PREFIX}{}", identity.as_str())
}

fn edit_commit_message(
    request: &PersonalEditRequest,
    identity: &ContentSha256,
    digest: &ContentSha256,
    revision: ConfigurationRevision,
) -> String {
    let run = request
        .originating_run_id
        .as_ref()
        .map(|run| run.as_str())
        .unwrap_or("none");
    format!(
        "Apply accepted PandaMUX personal configuration edit\n\nPandaMUX-Request-ID: {}\nPandaMUX-Request-Identity: {}\nPandaMUX-Request-Digest: {}\nPandaMUX-Caller: {}\nPandaMUX-Accepted-At-Ms: {}\nPandaMUX-Originating-Run: {}\nPandaMUX-Revision: {}\n",
        request.request_id,
        identity.as_str(),
        digest.as_str(),
        request.caller,
        request.accepted_at_ms,
        run,
        revision.get(),
    )
}

fn parse_trailers(message: &str) -> BTreeMap<String, String> {
    message
        .lines()
        .filter_map(|line| line.split_once(": "))
        .filter(|(key, _)| key.starts_with("PandaMUX-"))
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

fn projected_files(root: &Path) -> Result<BTreeMap<String, Vec<u8>>, PersonalRepoError> {
    let mut files = BTreeMap::new();
    for entry in fs::read_dir(root).map_err(|_| managed_path_error())? {
        let entry = entry.map_err(|_| managed_path_error())?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| managed_path_error())?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|_| managed_path_error())?;
        if metadata.file_type().is_symlink() {
            return Err(managed_path_error());
        }
        if name == ".git" {
            if !metadata.is_dir() {
                return Err(managed_path_error());
            }
            continue;
        }
        if matches!(name.as_str(), "agents" | "knowledge" | "prompts") && metadata.is_dir() {
            collect_projected_files(root, &path, &mut files)?;
            continue;
        }
        return Err(managed_path_error());
    }
    Ok(files)
}

fn collect_projected_files(
    root: &Path,
    directory: &Path,
    files: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), PersonalRepoError> {
    let metadata = fs::symlink_metadata(directory).map_err(|_| managed_path_error())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(managed_path_error());
    }
    for entry in fs::read_dir(directory).map_err(|_| managed_path_error())? {
        let entry = entry.map_err(|_| managed_path_error())?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|_| managed_path_error())?;
        if metadata.file_type().is_symlink() {
            return Err(managed_path_error());
        }
        if metadata.is_dir() {
            collect_projected_files(root, &path, files)?;
        } else if metadata.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|_| managed_path_error())?
                .to_string_lossy()
                .replace('\\', "/");
            validate_personal_path(&relative)?;
            files.insert(relative, fs::read(&path).map_err(|_| managed_path_error())?);
        } else {
            return Err(managed_path_error());
        }
    }
    Ok(())
}

fn create_private_directory(path: &Path) -> Result<(), PersonalRepoError> {
    fs::create_dir_all(path).map_err(|_| managed_path_error())?;
    let metadata = fs::symlink_metadata(path).map_err(|_| managed_path_error())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(managed_path_error());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| managed_path_error())?;
    }
    Ok(())
}

fn write_private_file(path: &Path, bytes: &[u8]) -> Result<(), PersonalRepoError> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|_| managed_path_error())?;
    file.write_all(bytes).map_err(|_| managed_path_error())?;
    file.sync_all().map_err(|_| managed_path_error())?;
    Ok(())
}

fn install_candidate(
    candidate: &Path,
    destination: &Path,
    recovery: &Path,
) -> Result<(), PersonalRepoError> {
    if destination.exists() && directory_is_empty(destination)? {
        fs::remove_dir(destination).map_err(|_| managed_path_error())?;
    }
    match fs::rename(candidate, destination) {
        Ok(()) => sync_directory(destination.parent().ok_or_else(managed_path_error)?),
        Err(_) if destination.exists() => move_owned_candidate_to_recovery(candidate, recovery),
        Err(_) => Err(managed_path_error()),
    }
}

fn move_owned_candidate_to_recovery(
    candidate: &Path,
    recovery: &Path,
) -> Result<(), PersonalRepoError> {
    create_private_directory(recovery)?;
    let name = candidate
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(managed_path_error)?;
    let destination = recovery.join(format!("{name}-{}", uuid::Uuid::new_v4().simple()));
    fs::rename(candidate, destination).map_err(|_| managed_path_error())?;
    sync_directory(recovery)
}

fn remove_owned_projection_candidate(candidate: &Path) -> Result<(), PersonalRepoError> {
    let name = candidate
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if !name.starts_with(PROJECTION_CANDIDATE_PREFIX) {
        return Err(managed_path_error());
    }
    fs::remove_dir_all(candidate).map_err(|_| managed_path_error())?;
    Ok(())
}

fn directory_is_empty(path: &Path) -> Result<bool, PersonalRepoError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| managed_path_error())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(managed_path_error());
    }
    Ok(fs::read_dir(path)
        .map_err(|_| managed_path_error())?
        .next()
        .is_none())
}

fn sync_directory(path: &Path) -> Result<(), PersonalRepoError> {
    #[cfg(unix)]
    fs::File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| managed_path_error())?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn sync_tree(path: &Path) -> Result<(), PersonalRepoError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| managed_path_error())?;
    if metadata.file_type().is_symlink() {
        return Err(managed_path_error());
    }
    if metadata.is_file() {
        return fs::File::open(path)
            .and_then(|file| file.sync_all())
            .map_err(|_| managed_path_error());
    }
    if !metadata.is_dir() {
        return Err(managed_path_error());
    }
    for entry in fs::read_dir(path).map_err(|_| managed_path_error())? {
        sync_tree(&entry.map_err(|_| managed_path_error())?.path())?;
    }
    sync_directory(path)
}

fn parse_utf8(bytes: &[u8]) -> Result<String, PersonalRepoError> {
    String::from_utf8(bytes.to_vec()).map_err(|_| corrupt_repository())
}

fn parse_trimmed(bytes: &[u8]) -> Result<String, PersonalRepoError> {
    Ok(parse_utf8(bytes)?.trim().to_string())
}

fn hash_part(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update(u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_be_bytes());
    hasher.update(bytes);
}

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn fail_if(
    actual: Option<PersonalRepoFailpoint>,
    expected: PersonalRepoFailpoint,
) -> Result<(), PersonalRepoError> {
    if actual == Some(expected) {
        return Err(PersonalRepoError {
            reason: "injected_personal_repo_failure",
            safe_message: "The personal repository test failpoint interrupted the operation"
                .to_string(),
            retryable: true,
        });
    }
    Ok(())
}

fn invalid_configuration() -> PersonalRepoError {
    PersonalRepoError {
        reason: "personal_configuration_invalid",
        safe_message: "The personal configuration did not pass validation".to_string(),
        retryable: false,
    }
}

fn corrupt_repository() -> PersonalRepoError {
    PersonalRepoError {
        reason: "personal_repo_recovering",
        safe_message: "The personal repository requires recovery before use".to_string(),
        retryable: true,
    }
}

fn managed_path_error() -> PersonalRepoError {
    PersonalRepoError {
        reason: "managed_path_unsafe",
        safe_message: "A managed personal repository path is unsafe".to_string(),
        retryable: false,
    }
}

fn personal_state_error() -> PersonalRepoError {
    PersonalRepoError {
        reason: "personal_state_failed",
        safe_message: "The personal repository state could not be reconciled".to_string(),
        retryable: true,
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn regular_files(root: &Path) -> BTreeMap<String, Vec<u8>> {
        fn collect(root: &Path, current: &Path, files: &mut BTreeMap<String, Vec<u8>>) {
            for entry in fs::read_dir(current).expect("read repository directory") {
                let path = entry.expect("repository entry").path();
                let metadata = fs::symlink_metadata(&path).expect("repository metadata");
                assert!(!metadata.file_type().is_symlink());
                if metadata.is_dir() {
                    collect(root, &path, files);
                } else if metadata.is_file() {
                    let relative = path
                        .strip_prefix(root)
                        .expect("repository relative path")
                        .to_string_lossy()
                        .replace('\\', "/");
                    files.insert(relative, fs::read(path).expect("repository file"));
                }
            }
        }

        let mut files = BTreeMap::new();
        collect(root, root, &mut files);
        files
    }

    fn service() -> (tempfile::TempDir, PersonalRepoService) {
        let temp = tempfile::tempdir().expect("temporary root");
        let paths = DataPaths::from_root(temp.path().join("app")).expect("paths");
        paths.ensure_private_roots().expect("private roots");
        let store = Store::in_memory().expect("store");
        (temp, PersonalRepoService::new(paths, store))
    }

    fn agent_bytes(id: &str, description: &str) -> Vec<u8> {
        format!(
            "---\nschemaVersion: 1\nkind: pandamux.agent\nid: {id}\nname: {id}\ndescription: {description}\nspecialties: []\ntier: fast\neffort: low\naccessMode: read_only\nrequiredCapabilities: []\nproviders: {{ prefer: [], avoid: [] }}\nenvironments: [local]\nknowledge: []\ntools: {{ allow: [], deny: [] }}\noutput: {{ kind: change_set }}\nverification: []\nconcurrency: 1\nbudget: {{ maxTokens: 1000, maxCostMicroUsd: 1000, maxRuntimeMs: 1000 }}\n---\nBody\n"
        )
        .into_bytes()
    }

    fn request(head: SourceCommitSha, request_id: &str, description: &str) -> PersonalEditRequest {
        PersonalEditRequest {
            request_id: request_id.to_string(),
            caller: "local-control".to_string(),
            accepted_at_ms: 2_000,
            originating_run_id: None,
            expected_head: head,
            files: BTreeMap::from([(
                "agents/agent-test.md".to_string(),
                agent_bytes("agent-test", description),
            )]),
        }
    }

    #[test]
    fn empty_bootstrap_creates_valid_main_and_concurrent_init_converges() {
        let temp = tempfile::tempdir().expect("temporary root");
        let paths = DataPaths::from_root(temp.path().join("app")).expect("paths");
        paths.ensure_private_roots().expect("private roots");
        let store = Store::in_memory().expect("store");
        let service = PersonalRepoService::new(paths.clone(), store.clone());
        let left = PersonalRepoService::new(paths.clone(), store.clone());
        let right = PersonalRepoService::new(paths, store);
        let first = std::thread::spawn(move || left.ensure_initialized());
        let second = std::thread::spawn(move || right.ensure_initialized());
        let first = first.join().expect("first join").expect("first init");
        let second = second.join().expect("second join").expect("second init");
        assert_eq!(first, second);
        assert_eq!(service.current_head_sha().expect("current head"), first);
    }

    #[test]
    fn accepted_edit_is_idempotent_and_conflicting_identity_fails() {
        let (_temp, service) = service();
        let head = service.ensure_initialized().expect("initialize");
        let edit_request = request(head, "request-one", "First description");
        let first = service.apply_edit(&edit_request).expect("apply edit");
        let replay = service.apply_edit(&edit_request).expect("replay edit");
        assert_eq!(replay.source_commit_sha, first.source_commit_sha);
        assert!(replay.duplicate);

        let mut conflict = edit_request;
        conflict.files.insert(
            "agents/agent-test.md".to_string(),
            agent_bytes("agent-test", "Changed bytes"),
        );
        let error = service
            .apply_edit(&conflict)
            .expect_err("conflicting identity must fail");
        assert_eq!(error.reason, "request_identity_conflict");

        let head = service.current_head_sha().expect("head after first edit");
        let mut run_request = request(head, "request-run", "Run identity");
        run_request.originating_run_id = Some(RunId::new("run-one"));
        service.apply_edit(&run_request).expect("apply run edit");
        run_request.originating_run_id = Some(RunId::new("run-two"));
        assert_eq!(
            service
                .apply_edit(&run_request)
                .expect_err("changed originating run must conflict")
                .reason,
            "request_identity_conflict"
        );
    }

    #[test]
    fn competing_expected_head_writers_have_one_winner() {
        let (_temp, service) = service();
        let head = service.ensure_initialized().expect("initialize");
        let first_request = request(head.clone(), "request-left", "Left");
        let second_request = request(head, "request-right", "Right");
        let left = service.clone();
        let right = service.clone();
        let first = std::thread::spawn(move || left.apply_edit(&first_request));
        let second = std::thread::spawn(move || right.apply_edit(&second_request));
        let results = [
            first.join().expect("left join"),
            second.join().expect("right join"),
        ];
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter_map(|result| result.as_ref().err())
                .next()
                .expect("one loser")
                .reason,
            "personal_head_changed"
        );
    }

    #[test]
    fn every_edit_failpoint_preserves_head_and_retry_applies_once() {
        for failpoint in [
            PersonalRepoFailpoint::AfterObjects,
            PersonalRepoFailpoint::AfterCommit,
            PersonalRepoFailpoint::BeforeRef,
            PersonalRepoFailpoint::AfterRef,
            PersonalRepoFailpoint::BeforeReconciliation,
        ] {
            let (_temp, service) = service();
            let head = service.ensure_initialized().expect("initialize");
            let request = request(head.clone(), "request-failpoint", "Failpoint");
            assert!(
                service
                    .apply_edit_with_failpoint(&request, Some(failpoint))
                    .is_err()
            );
            service
                .current_head_sha()
                .expect("valid head after failure");
            let result = service.apply_edit(&request).expect("retry edit");
            assert_eq!(service.history(10).expect("history").len(), 1);
            assert_eq!(
                result.source_commit_sha,
                service.current_head_sha().unwrap()
            );
        }
    }

    #[test]
    fn initialization_failpoints_recover_without_an_invalid_active_repo() {
        for failpoint in [
            PersonalRepoFailpoint::AfterDirectory,
            PersonalRepoFailpoint::AfterObjects,
            PersonalRepoFailpoint::AfterCommit,
            PersonalRepoFailpoint::BeforeRef,
            PersonalRepoFailpoint::AfterRef,
            PersonalRepoFailpoint::BeforeReconciliation,
        ] {
            let (_temp, service) = service();
            assert!(
                service
                    .ensure_initialized_with_failpoint(Some(failpoint))
                    .is_err()
            );
            service
                .ensure_initialized()
                .expect("recover initialization");
            service.current_head_sha().expect("valid recovered head");
        }
    }

    #[test]
    fn secret_and_runtime_rejections_create_no_reachable_commit() {
        let (_temp, service) = service();
        let head = service.ensure_initialized().expect("initialize");
        let before = service.history(100).expect("history before");
        let git_dir = service.git_dir();
        let objects_before = regular_files(&git_dir.join("objects"));
        let state_before = service
            .store
            .get_personal_repo_state()
            .expect("state before rejection");
        let secret_sentinel = "Bearer abcdefghijklmnopqrstuvwxyz";
        let mut secret = request(head.clone(), "request-secret", "safe");
        secret.files.insert(
            "prompts/secret.md".to_string(),
            format!("---\nschemaVersion: 1\nkind: pandamux.prompt\nid: secret\npurpose: intake\n---\n{secret_sentinel}\n").into_bytes(),
        );
        let secret_error = service.apply_edit(&secret).expect_err("secret must fail");
        assert_eq!(secret_error.reason, "secret_or_runtime_data_rejected");
        assert!(!secret_error.safe_message.contains(secret_sentinel));
        let mut runtime = request(head.clone(), "request-runtime", "safe");
        runtime.files = BTreeMap::from([("run/state.json".to_string(), b"{}".to_vec())]);
        let runtime_error = service
            .apply_edit(&runtime)
            .expect_err("runtime path must fail");
        assert_eq!(runtime_error.reason, "secret_or_runtime_data_rejected");
        let mut unsafe_run = request(head, "request-run-secret", "safe");
        unsafe_run.originating_run_id = Some(RunId::new(format!("run-{secret_sentinel}")));
        let run_error = service
            .apply_edit(&unsafe_run)
            .expect_err("sensitive run metadata must fail");
        assert_eq!(run_error.reason, "secret_or_runtime_data_rejected");
        assert!(!run_error.safe_message.contains(secret_sentinel));
        assert_eq!(regular_files(&git_dir.join("objects")), objects_before);
        assert_eq!(
            service
                .store
                .get_personal_repo_state()
                .expect("state after rejection"),
            state_before
        );
        assert_eq!(service.history(100).expect("history after"), before);
        assert!(!git_dir.join("logs").exists());
    }

    #[test]
    fn case_collisions_fail_before_document_parsing_or_object_creation() {
        let (_temp, service) = service();
        let head = service.ensure_initialized().expect("initialize");
        let objects_before = regular_files(&service.git_dir().join("objects"));
        let mut collision = request(head, "request-collision", "safe");
        collision.files.insert(
            "agents/AGENT-TEST.md".to_string(),
            b"not even a valid document".to_vec(),
        );
        assert_eq!(
            service
                .apply_edit(&collision)
                .expect_err("case collision must fail")
                .reason,
            "case_colliding_configuration_path"
        );
        assert_eq!(
            regular_files(&service.git_dir().join("objects")),
            objects_before
        );

        for (request_id, path, bytes) in [
            (
                "request-control-path",
                "agents/agent-test\n.md",
                agent_bytes("agent-test", "safe"),
            ),
            (
                "request-device-path",
                "agents/con/agent-test.md",
                agent_bytes("agent-test", "safe"),
            ),
            (
                "request-oversized",
                "agents/agent-test.md",
                vec![b'x'; pandamux_orchestrator::schema::MAX_MARKDOWN_FILE_BYTES + 1],
            ),
        ] {
            let unsafe_request = PersonalEditRequest {
                request_id: request_id.to_string(),
                caller: "local-control".to_string(),
                accepted_at_ms: 2_000,
                originating_run_id: None,
                expected_head: service.current_head_sha().expect("unchanged head"),
                files: BTreeMap::from([(path.to_string(), bytes)]),
            };
            assert_eq!(
                service
                    .apply_edit(&unsafe_request)
                    .expect_err("unsafe path or bound must fail before Git")
                    .reason,
                "personal_configuration_invalid"
            );
            assert_eq!(
                regular_files(&service.git_dir().join("objects")),
                objects_before
            );
        }

        for segment in ["CON", "aux.txt", "COM1", "lpt9.log", "trailing."] {
            assert!(portable_path_segment_is_unsafe(segment));
        }
        assert!(!portable_path_segment_is_unsafe("computer"));
    }

    #[test]
    fn exact_crlf_bytes_are_retained_without_running_hooks() {
        let (_temp, service) = service();
        let head = service.ensure_initialized().expect("initialize");
        let lf = agent_bytes("agent-test", "CRLF document");
        let crlf = String::from_utf8(lf)
            .expect("UTF-8 fixture")
            .replace('\n', "\r\n")
            .into_bytes();
        let mut edit = request(head, "request-crlf", "unused");
        edit.files = BTreeMap::from([("agents/agent-test.md".to_string(), crlf.clone())]);

        #[cfg(unix)]
        let hook_marker = {
            use std::os::unix::fs::PermissionsExt;
            let marker = service.paths.root().join("hook-ran");
            let hook = service.git_dir().join("hooks/pre-commit");
            fs::write(&hook, format!("#!/bin/sh\ntouch '{}'\n", marker.display()))
                .expect("write hook");
            fs::set_permissions(&hook, fs::Permissions::from_mode(0o700))
                .expect("make hook executable");
            marker
        };

        let result = service.apply_edit(&edit).expect("apply exact-byte edit");
        let snapshot = service
            .read_snapshot(&service.git_dir(), result.source_commit_sha.as_str())
            .expect("read committed snapshot");
        assert_eq!(snapshot.get("agents/agent-test.md"), Some(&crlf));
        #[cfg(unix)]
        assert!(!hook_marker.exists());
    }

    #[test]
    fn projection_recovery_moves_owned_candidates_and_preserves_unknown_entries() {
        let (_temp, service) = service();
        let head = service.ensure_initialized().expect("initialize");
        let interrupted = service
            .paths
            .root()
            .join(format!("{PROJECTION_CANDIDATE_PREFIX}interrupted"));
        create_private_directory(&interrupted).expect("interrupted projection candidate");
        write_private_file(&interrupted.join("owned"), b"candidate").expect("candidate file");
        assert_eq!(
            service.current_head_sha().expect("recover projection"),
            head
        );
        assert!(!interrupted.exists());
        assert!(
            fs::read_dir(service.paths.recovery())
                .expect("recovery directory")
                .filter_map(Result::ok)
                .any(|entry| entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(PROJECTION_CANDIDATE_PREFIX))
        );

        let unknown = service.paths.personal().join("unknown-runtime-state");
        fs::write(&unknown, b"preserve me").expect("unknown projection entry");
        let error = service
            .current_head_sha()
            .expect_err("unknown projection entry must fail closed");
        assert_eq!(error.reason, "managed_path_unsafe");
        assert_eq!(
            fs::read(&unknown).expect("unknown entry retained"),
            b"preserve me"
        );
    }

    #[test]
    fn revert_creates_a_child_commit_and_restores_exact_tree() {
        let (_temp, service) = service();
        let bootstrap = service.ensure_initialized().expect("initialize");
        let first = service
            .apply_edit(&request(bootstrap, "request-first", "First"))
            .expect("first edit");
        let second = service
            .apply_edit(&request(
                first.source_commit_sha.clone(),
                "request-second",
                "Second",
            ))
            .expect("second edit");
        let reverted = service
            .revert(
                "request-revert".to_string(),
                "local-control".to_string(),
                4_000,
                None,
                second.source_commit_sha.clone(),
                first.source_commit_sha.clone(),
            )
            .expect("revert");
        assert_eq!(reverted.parent_source_commit_sha, second.source_commit_sha);
        assert!(
            service
                .diff(&first.source_commit_sha, &reverted.source_commit_sha)
                .expect("exact diff")
                .added
                .is_empty()
        );
        let diff = service
            .diff(&first.source_commit_sha, &reverted.source_commit_sha)
            .expect("exact diff");
        assert!(diff.modified.is_empty());
        assert!(diff.deleted.is_empty());
        assert_eq!(service.history(10).expect("history").len(), 3);
    }
}
