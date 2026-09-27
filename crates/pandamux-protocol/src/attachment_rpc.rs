use pandamux_core::{AttachmentRecord, ThreadId};
use serde::{Deserialize, Serialize};

/// Parameters for `attachment.import_path` / `attachment.importPath`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentImportPathParams {
    pub thread_id: ThreadId,
    pub path: String,
}

/// Result of `attachment.import_path`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentImportResult {
    pub attachment: AttachmentRecord,
}

/// Parameters for `attachment.put` (chunked base64 transfer).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentPutChunkParams {
    pub thread_id: ThreadId,
    pub id: String,
    pub chunk_index: u32,
    pub total_chunks: u32,
    pub data_base64: String,
    pub mime_type: String,
    pub file_name: String,
}

/// Result of `attachment.put`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentPutResult {
    pub attachment: Option<AttachmentRecord>,
    pub is_complete: bool,
    pub received_chunks: u32,
    pub total_chunks: u32,
}

/// Parameters for `attachment.list`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentListParams {
    pub thread_id: ThreadId,
}

/// Result of `attachment.list`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentListResult {
    pub attachments: Vec<AttachmentRecord>,
}

/// Parameters for `attachment.get`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentGetParams {
    pub thread_id: ThreadId,
    pub id: String,
}

/// Result of `attachment.get`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentGetResult {
    pub attachment: AttachmentRecord,
}
