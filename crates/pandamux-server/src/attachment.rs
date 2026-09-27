use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

use pandamux_core::{AttachmentRecord, ThreadId, detect_mime_type, validate_attachment_size};
use pandamux_protocol::{
    AttachmentGetResult, AttachmentImportResult, AttachmentListResult, AttachmentPutChunkParams,
    AttachmentPutResult, RpcError,
};

use crate::store::Store;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Tracks in-flight chunked uploads for an attachment.
struct InFlightUpload {
    thread_id: ThreadId,
    id: String,
    file_name: String,
    mime_type: String,
    total_chunks: u32,
    chunks: HashMap<u32, Vec<u8>>,
    accumulated_bytes: u64,
}

/// Manages local attachment storage, chunked assembly, and size cap validations.
#[derive(Clone)]
pub struct AttachmentManager {
    base_dir: PathBuf,
    in_flight: Arc<Mutex<HashMap<String, InFlightUpload>>>,
}

impl AttachmentManager {
    /// Creates a new `AttachmentManager` storing files under `base_dir`.
    pub fn new(base_dir: PathBuf) -> Self {
        Self {
            base_dir,
            in_flight: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Returns the storage directory for attachments of a specific thread.
    pub fn thread_dir(&self, thread_id: &ThreadId) -> PathBuf {
        self.base_dir.join(thread_id.as_str())
    }

    /// Imports a file from a local filesystem path into PandaMUX's attachment storage.
    pub fn import_path(
        &self,
        store: &Store,
        thread_id: &ThreadId,
        path_str: &str,
    ) -> Result<AttachmentImportResult, RpcError> {
        let src_path = Path::new(path_str);
        if !src_path.exists() {
            return Err(RpcError::invalid_params(format!(
                "Attachment file does not exist: {path_str}"
            )));
        }
        if !src_path.is_file() {
            return Err(RpcError::invalid_params(format!(
                "Attachment path is not a file: {path_str}"
            )));
        }

        let meta = fs::metadata(src_path)
            .map_err(|e| RpcError::internal_error(format!("Failed to read file metadata: {e}")))?;
        let size_bytes = meta.len();

        let file_name = src_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("attachment")
            .to_string();

        let mime_type = detect_mime_type(&file_name).to_string();

        // Enforce size cap
        validate_attachment_size(&mime_type, size_bytes)
            .map_err(|e| RpcError::invalid_params(e.to_string()))?;

        // Server-generated attachment ID
        let id = format!("att-{}", uuid::Uuid::new_v4().simple());
        let target_dir = self.thread_dir(thread_id);
        fs::create_dir_all(&target_dir).map_err(|e| {
            RpcError::internal_error(format!("Failed to create attachment directory: {e}"))
        })?;

        let target_path = target_dir.join(&id);
        fs::copy(src_path, &target_path).map_err(|e| {
            RpcError::internal_error(format!("Failed to copy attachment file: {e}"))
        })?;

        let record = AttachmentRecord {
            id,
            thread_id: thread_id.clone(),
            file_name,
            mime_type,
            size_bytes,
            file_path: target_path.to_string_lossy().to_string(),
            created_at_ms: now_ms(),
        };

        store
            .save_attachment(&record)
            .map_err(|e| RpcError::internal_error(e.to_string()))?;

        Ok(AttachmentImportResult { attachment: record })
    }

    /// Handles a chunk of an attachment upload (e.g. from clipboard or remote sync).
    pub async fn put_chunk(
        &self,
        store: &Store,
        params: AttachmentPutChunkParams,
    ) -> Result<AttachmentPutResult, RpcError> {
        if params.total_chunks == 0 {
            return Err(RpcError::invalid_params(
                "totalChunks must be greater than 0",
            ));
        }
        if params.chunk_index >= params.total_chunks {
            return Err(RpcError::invalid_params(format!(
                "chunkIndex {} out of bounds for totalChunks {}",
                params.chunk_index, params.total_chunks
            )));
        }

        let chunk_bytes = BASE64
            .decode(params.data_base64.as_bytes())
            .map_err(|e| RpcError::invalid_params(format!("Invalid base64 chunk data: {e}")))?;

        let upload_key = format!("{}:{}", params.thread_id.as_str(), params.id);
        let mut guard = self.in_flight.lock().await;

        let entry = guard
            .entry(upload_key.clone())
            .or_insert_with(|| InFlightUpload {
                thread_id: params.thread_id.clone(),
                id: params.id.clone(),
                file_name: params.file_name.clone(),
                mime_type: params.mime_type.clone(),
                total_chunks: params.total_chunks,
                chunks: HashMap::new(),
                accumulated_bytes: 0,
            });

        // Track accumulated size and check cap
        let chunk_len = chunk_bytes.len() as u64;
        if !entry.chunks.contains_key(&params.chunk_index) {
            entry.accumulated_bytes += chunk_len;
            validate_attachment_size(&entry.mime_type, entry.accumulated_bytes)
                .map_err(|e| RpcError::invalid_params(e.to_string()))?;
            entry.chunks.insert(params.chunk_index, chunk_bytes);
        }

        let received_count = entry.chunks.len() as u32;
        let is_complete = received_count == entry.total_chunks;

        if is_complete {
            // Assemble all chunks in order
            let mut assembled = Vec::with_capacity(entry.accumulated_bytes as usize);
            for idx in 0..entry.total_chunks {
                if let Some(c) = entry.chunks.get(&idx) {
                    assembled.extend_from_slice(c);
                } else {
                    return Err(RpcError::internal_error(format!(
                        "Missing chunk index {idx}"
                    )));
                }
            }

            let size_bytes = assembled.len() as u64;
            validate_attachment_size(&entry.mime_type, size_bytes)
                .map_err(|e| RpcError::invalid_params(e.to_string()))?;

            let target_dir = self.thread_dir(&entry.thread_id);
            fs::create_dir_all(&target_dir).map_err(|e| {
                RpcError::internal_error(format!("Failed to create attachment directory: {e}"))
            })?;

            let target_path = target_dir.join(&entry.id);
            fs::write(&target_path, &assembled).map_err(|e| {
                RpcError::internal_error(format!("Failed to write assembled attachment: {e}"))
            })?;

            let record = AttachmentRecord {
                id: entry.id.clone(),
                thread_id: entry.thread_id.clone(),
                file_name: entry.file_name.clone(),
                mime_type: entry.mime_type.clone(),
                size_bytes,
                file_path: target_path.to_string_lossy().to_string(),
                created_at_ms: now_ms(),
            };

            store
                .save_attachment(&record)
                .map_err(|e| RpcError::internal_error(e.to_string()))?;

            // Clean up in-flight state
            guard.remove(&upload_key);

            Ok(AttachmentPutResult {
                attachment: Some(record),
                is_complete: true,
                received_chunks: received_count,
                total_chunks: params.total_chunks,
            })
        } else {
            Ok(AttachmentPutResult {
                attachment: None,
                is_complete: false,
                received_chunks: received_count,
                total_chunks: params.total_chunks,
            })
        }
    }

    /// Lists attachments registered for a thread.
    pub fn list(
        &self,
        store: &Store,
        thread_id: &ThreadId,
    ) -> Result<AttachmentListResult, RpcError> {
        let attachments = store
            .list_attachments(thread_id)
            .map_err(|e| RpcError::internal_error(e.to_string()))?;
        Ok(AttachmentListResult { attachments })
    }

    /// Retrieves an attachment record by ID.
    pub fn get(
        &self,
        store: &Store,
        thread_id: &ThreadId,
        id: &str,
    ) -> Result<AttachmentGetResult, RpcError> {
        let attachment = store
            .get_attachment(thread_id, id)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(|| {
                RpcError::invalid_params(format!(
                    "Attachment '{id}' not found for thread '{}'",
                    thread_id.as_str()
                ))
            })?;
        Ok(AttachmentGetResult { attachment })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn dummy_thread(id: &str) -> pandamux_core::Thread {
        pandamux_core::Thread {
            id: ThreadId::from(id),
            project_id: None,
            environment_id: pandamux_core::EnvironmentId::from("env-local"),
            parent_thread_id: None,
            title: "Test".to_string(),
            provider_instance_id: pandamux_core::ProviderInstanceId::from("prov-mock"),
            model: "mock".to_string(),
            effort: None,
            access_mode: Default::default(),
            workspace: pandamux_core::ThreadWorkspace {
                cwd: "/tmp".to_string(),
                worktree: None,
            },
            status: pandamux_core::ThreadStatus::Idle,
            agent: None,
            origin: Default::default(),
            created_at_ms: 1000,
            updated_at_ms: 1000,
        }
    }

    #[tokio::test]
    async fn test_import_path_and_get() {
        let temp_dir = tempdir().expect("tempdir");
        let store = Store::in_memory().expect("store in_memory");
        let manager = AttachmentManager::new(temp_dir.path().to_path_buf());
        let thread_id = ThreadId::from("thread-import-1");
        store
            .save_thread(&dummy_thread("thread-import-1"))
            .expect("save thread");

        // Create sample file
        let sample_file = temp_dir.path().join("sample.txt");
        fs::write(&sample_file, b"Hello Pandamux Attachments!").expect("write sample");

        let import_res = manager
            .import_path(&store, &thread_id, sample_file.to_str().unwrap())
            .expect("import path");
        assert_eq!(import_res.attachment.file_name, "sample.txt");
        assert_eq!(import_res.attachment.mime_type, "text/plain");
        assert_eq!(import_res.attachment.size_bytes, 27);

        // Verify stored file exists on disk
        assert!(Path::new(&import_res.attachment.file_path).exists());

        // Get by ID
        let get_res = manager
            .get(&store, &thread_id, &import_res.attachment.id)
            .expect("get attachment");
        assert_eq!(get_res.attachment.id, import_res.attachment.id);

        // List attachments
        let list_res = manager.list(&store, &thread_id).expect("list attachments");
        assert_eq!(list_res.attachments.len(), 1);
    }

    #[tokio::test]
    async fn test_chunked_put_and_assembly() {
        let temp_dir = tempdir().expect("tempdir");
        let store = Store::in_memory().expect("store in_memory");
        let manager = AttachmentManager::new(temp_dir.path().to_path_buf());
        let thread_id = ThreadId::from("thread-put-1");
        store
            .save_thread(&dummy_thread("thread-put-1"))
            .expect("save thread");

        let full_data = b"Chunk One Content - Chunk Two Content - Done!";
        let chunk1_bytes = &full_data[..20];
        let chunk2_bytes = &full_data[20..];

        let chunk1_b64 = BASE64.encode(chunk1_bytes);
        let chunk2_b64 = BASE64.encode(chunk2_bytes);

        // Send chunk 0 of 2
        let res1 = manager
            .put_chunk(
                &store,
                AttachmentPutChunkParams {
                    thread_id: thread_id.clone(),
                    id: "att-clip-1".into(),
                    chunk_index: 0,
                    total_chunks: 2,
                    data_base64: chunk1_b64,
                    mime_type: "text/plain".into(),
                    file_name: "pasted.txt".into(),
                },
            )
            .await
            .expect("put chunk 0");
        assert!(!res1.is_complete);
        assert_eq!(res1.received_chunks, 1);
        assert!(res1.attachment.is_none());

        // Send chunk 1 of 2
        let res2 = manager
            .put_chunk(
                &store,
                AttachmentPutChunkParams {
                    thread_id: thread_id.clone(),
                    id: "att-clip-1".into(),
                    chunk_index: 1,
                    total_chunks: 2,
                    data_base64: chunk2_b64,
                    mime_type: "text/plain".into(),
                    file_name: "pasted.txt".into(),
                },
            )
            .await
            .expect("put chunk 1");
        assert!(res2.is_complete);
        assert_eq!(res2.received_chunks, 2);
        assert!(res2.attachment.is_some());

        let att = res2.attachment.unwrap();
        assert_eq!(att.id, "att-clip-1");
        assert_eq!(att.size_bytes, full_data.len() as u64);

        // Verify content on disk
        let disk_content = fs::read(&att.file_path).expect("read file");
        assert_eq!(disk_content, full_data);
    }
}
