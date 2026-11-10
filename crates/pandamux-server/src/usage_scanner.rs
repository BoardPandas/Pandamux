use crate::store::{Store, StoreError, UsageQuarantineRecord, UsageScannerCursor};
use pandamux_core::{
    EnvironmentId, NodeStoreId, ProviderInstanceId, ProviderKind, SubscriptionKey, UsageFactKind,
    UsageObservation, UsageObservationId, UsageProvider, UsageSourceKind,
};
use pandamux_protocol::{UsageScannerDiagnostic, UsageSourceCoverageState};
use pandamux_providers::{
    ParsedTranscriptUsage, TranscriptParser, TranscriptProvider, TranscriptRecordOutcome,
    resolve_instance_profile_dir,
};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::{self, File, Metadata};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const PREFIX_WINDOW_BYTES: u64 = 64;

#[derive(Clone, Debug)]
pub struct UsageScannerLimits {
    pub max_directories: usize,
    pub max_files: usize,
    pub max_file_bytes: u64,
    pub max_bytes_per_scan: u64,
    pub max_record_bytes: usize,
    pub max_records_per_file: usize,
    pub max_managed_turns_per_scan: u32,
    pub max_quarantines_per_file: usize,
    pub max_scan_duration: Duration,
}

impl Default for UsageScannerLimits {
    fn default() -> Self {
        Self {
            max_directories: 512,
            max_files: 4_096,
            max_file_bytes: 256 * 1024 * 1024,
            max_bytes_per_scan: 64 * 1024 * 1024,
            max_record_bytes: 1024 * 1024,
            max_records_per_file: 20_000,
            max_managed_turns_per_scan: 1_000,
            max_quarantines_per_file: 256,
            max_scan_duration: Duration::from_secs(10),
        }
    }
}

#[derive(Clone, Debug)]
pub struct UsageScannerRuntimeConfig {
    pub limits: UsageScannerLimits,
    pub periodic_interval: Duration,
    pub periodic_jitter: Duration,
    pub trigger_ttl: Duration,
    pub trigger_queue_capacity: usize,
    pub shutdown_poll_interval: Duration,
}

impl Default for UsageScannerRuntimeConfig {
    fn default() -> Self {
        Self {
            limits: UsageScannerLimits::default(),
            periodic_interval: Duration::from_secs(15 * 60),
            periodic_jitter: Duration::from_secs(90),
            trigger_ttl: Duration::from_secs(5 * 60),
            trigger_queue_capacity: 8,
            shutdown_poll_interval: Duration::from_millis(100),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsageScanTrigger {
    Startup,
    TurnCompleted,
    Periodic,
    Manual,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PlatformFileIdentity {
    Unix { device: u64, inode: u64 },
    Windows { volume_serial: u32, file_index: u64 },
}

impl PlatformFileIdentity {
    pub fn stable_key(&self) -> String {
        match self {
            Self::Unix { device, inode } => format!("unix:{device}:{inode}"),
            Self::Windows {
                volume_serial,
                file_index,
            } => format!("windows:{volume_serial}:{file_index}"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct UsageScanSource {
    pub environment_id: EnvironmentId,
    pub provider_instance_id: ProviderInstanceId,
    pub provider: UsageProvider,
    pub subscription_key: Option<SubscriptionKey>,
    pub source_kind: UsageSourceKind,
    pub parser_provider: TranscriptProvider,
    pub root: PathBuf,
    pub mapping_error: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UsageScanRunReport {
    pub sources_scanned: u64,
    pub files_scanned: u64,
    pub bytes_scanned: u64,
    pub inserted: u64,
    pub duplicates: u64,
    pub reconciled: u64,
    pub quarantined: u64,
    pub safe_error_code: Option<String>,
}

#[derive(Clone)]
pub struct UsageScannerHandle {
    sender: SyncSender<QueuedTrigger>,
}

impl UsageScannerHandle {
    pub fn trigger(&self, trigger: UsageScanTrigger) -> bool {
        let request = QueuedTrigger {
            trigger,
            enqueued_at: Instant::now(),
        };
        match self.sender.try_send(request) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => false,
        }
    }
}

pub struct UsageScannerService {
    handle: UsageScannerHandle,
    shutdown: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    last_report: Arc<Mutex<Option<UsageScanRunReport>>>,
    drain_budget: Duration,
}

impl UsageScannerService {
    pub fn start(
        store: Store,
        environment_id: EnvironmentId,
        data_dir: PathBuf,
        config: UsageScannerRuntimeConfig,
    ) -> io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(config.trigger_queue_capacity.max(1));
        let handle = UsageScannerHandle { sender };
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_shutdown = shutdown.clone();
        let last_report = Arc::new(Mutex::new(None));
        let worker_report = last_report.clone();
        let drain_budget = config
            .limits
            .max_scan_duration
            .saturating_mul(2)
            .saturating_add(config.shutdown_poll_interval);
        let worker = std::thread::Builder::new()
            .name("pandamux-usage-scanner".to_string())
            .spawn(move || {
                scanner_worker(
                    store,
                    environment_id,
                    data_dir,
                    config,
                    receiver,
                    worker_shutdown,
                    worker_report,
                );
            })?;
        let service = Self {
            handle,
            shutdown,
            worker: Some(worker),
            last_report,
            drain_budget,
        };
        let _ = service.handle.trigger(UsageScanTrigger::Startup);
        Ok(service)
    }

    pub fn handle(&self) -> UsageScannerHandle {
        self.handle.clone()
    }

    pub fn last_report(&self) -> Option<UsageScanRunReport> {
        self.last_report
            .lock()
            .expect("usage scanner report lock")
            .clone()
    }

    pub fn drain_budget(&self) -> Duration {
        self.drain_budget
    }

    pub async fn shutdown(mut self) -> io::Result<()> {
        self.shutdown.store(true, Ordering::Release);
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };
        tokio::task::spawn_blocking(move || worker.join())
            .await
            .map_err(|error| io::Error::other(format!("usage scanner join failed: {error}")))?
            .map_err(|_| io::Error::other("usage scanner worker panicked"))
    }
}

impl Drop for UsageScannerService {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[derive(Clone, Copy)]
struct QueuedTrigger {
    trigger: UsageScanTrigger,
    enqueued_at: Instant,
}

fn scanner_worker(
    store: Store,
    environment_id: EnvironmentId,
    data_dir: PathBuf,
    config: UsageScannerRuntimeConfig,
    receiver: Receiver<QueuedTrigger>,
    shutdown: Arc<AtomicBool>,
    last_report: Arc<Mutex<Option<UsageScanRunReport>>>,
) {
    let jitter = deterministic_jitter(&environment_id, config.periodic_jitter);
    let mut next_periodic = Instant::now()
        .checked_add(config.periodic_interval)
        .and_then(|value| value.checked_add(jitter))
        .unwrap_or_else(Instant::now);

    while !shutdown.load(Ordering::Acquire) {
        let now = Instant::now();
        let until_periodic = next_periodic.saturating_duration_since(now);
        let timeout = until_periodic.min(config.shutdown_poll_interval);
        let mut should_scan = false;
        match receiver.recv_timeout(timeout) {
            Ok(request) => {
                should_scan = request.enqueued_at.elapsed() <= config.trigger_ttl;
                while let Ok(next) = receiver.try_recv() {
                    should_scan |= next.enqueued_at.elapsed() <= config.trigger_ttl;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if Instant::now() >= next_periodic {
            should_scan = true;
            next_periodic = Instant::now()
                .checked_add(config.periodic_interval)
                .and_then(|value| value.checked_add(jitter))
                .unwrap_or_else(Instant::now);
        }
        if should_scan {
            let report =
                scan_configured_sources(&store, &environment_id, &data_dir, &config.limits)
                    .unwrap_or_else(|error| UsageScanRunReport {
                        safe_error_code: Some(store_error_code(&error).to_string()),
                        ..UsageScanRunReport::default()
                    });
            *last_report.lock().expect("usage scanner report lock") = Some(report);
        }
    }

    let mut had_pending = false;
    while let Ok(request) = receiver.try_recv() {
        let _ = request.trigger;
        had_pending |= request.enqueued_at.elapsed() <= config.trigger_ttl;
    }
    if had_pending {
        let report = scan_configured_sources(&store, &environment_id, &data_dir, &config.limits)
            .unwrap_or_else(|error| UsageScanRunReport {
                safe_error_code: Some(store_error_code(&error).to_string()),
                ..UsageScanRunReport::default()
            });
        *last_report.lock().expect("usage scanner report lock") = Some(report);
    }
}

fn deterministic_jitter(environment_id: &EnvironmentId, maximum: Duration) -> Duration {
    let maximum_ms = maximum.as_millis();
    if maximum_ms == 0 {
        return Duration::ZERO;
    }
    let digest = Sha256::digest(environment_id.as_str().as_bytes());
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    let value = u64::from_le_bytes(bytes);
    let modulus = u64::try_from(maximum_ms).unwrap_or(u64::MAX);
    Duration::from_millis(value % modulus.max(1))
}

pub fn configured_usage_sources(
    store: &Store,
    environment_id: &EnvironmentId,
    data_dir: &Path,
) -> Result<Vec<UsageScanSource>, StoreError> {
    let settings = store.get_settings()?.unwrap_or_default();
    let environment = settings
        .environments
        .iter()
        .find(|environment| &environment.id == environment_id);
    let mut sources = Vec::new();
    for config in settings.providers {
        if config
            .environment_id
            .as_ref()
            .is_some_and(|configured| configured != environment_id)
        {
            continue;
        }
        let (parser_provider, source_kind, provider, relative_root) = match config.provider {
            ProviderKind::Claude => (
                TranscriptProvider::Claude,
                UsageSourceKind::ClaudeTranscript,
                UsageProvider::Claude,
                "projects",
            ),
            ProviderKind::Codex => (
                TranscriptProvider::Codex,
                UsageSourceKind::CodexTranscript,
                UsageProvider::Codex,
                "sessions",
            ),
            _ => continue,
        };
        let override_config = environment
            .and_then(|environment| environment.provider_overrides.get(config.id.as_str()));
        let resolved = config.resolve_for_environment(environment_id, override_config);
        let profile_dir = resolve_instance_profile_dir(data_dir, &resolved);
        let mapping = store.usage_subscription_for_provider(environment_id, &resolved.id)?;
        let (subscription_key, mapping_error) = match mapping {
            Some((subscription_key, mapped_provider)) if mapped_provider == provider => {
                (Some(subscription_key), None)
            }
            Some(_) => (None, Some("subscription_provider_mismatch".to_string())),
            None => (None, Some("subscription_mapping_missing".to_string())),
        };
        sources.push(UsageScanSource {
            environment_id: environment_id.clone(),
            provider_instance_id: resolved.id,
            provider,
            subscription_key,
            source_kind,
            parser_provider,
            root: profile_dir.join(relative_root),
            mapping_error,
        });
    }
    sources.sort_by(|left, right| {
        left.provider_instance_id
            .cmp(&right.provider_instance_id)
            .then_with(|| left.source_kind.cmp(&right.source_kind))
    });
    Ok(sources)
}

pub fn scan_configured_sources(
    store: &Store,
    environment_id: &EnvironmentId,
    data_dir: &Path,
    limits: &UsageScannerLimits,
) -> Result<UsageScanRunReport, StoreError> {
    let mut report = UsageScanRunReport::default();
    let deadline = Instant::now()
        .checked_add(limits.max_scan_duration)
        .unwrap_or_else(Instant::now);
    let managed_limit = limits.max_managed_turns_per_scan.clamp(1, 10_000);
    let managed = store.backfill_managed_turn_usage(environment_id, managed_limit)?;
    report.inserted = managed.batch.inserted;
    report.duplicates = managed.batch.duplicates;
    report.reconciled = managed.batch.reconciled;
    if managed.unmapped > 0 {
        report.safe_error_code = Some("managed_subscription_mapping_missing".to_string());
    } else if managed.invalid > 0 {
        report.safe_error_code = Some("managed_usage_invalid".to_string());
    }
    let sources = configured_usage_sources(store, environment_id, data_dir)?;
    for source in sources {
        let source_report = scan_source_until(store, &source, limits, deadline)?;
        report.sources_scanned = checked_add(report.sources_scanned, 1, "source count")?;
        report.files_scanned = checked_add(
            report.files_scanned,
            source_report.files_scanned,
            "file count",
        )?;
        report.bytes_scanned = checked_add(
            report.bytes_scanned,
            source_report.bytes_scanned,
            "byte count",
        )?;
        report.inserted = checked_add(report.inserted, source_report.inserted, "insert count")?;
        report.duplicates = checked_add(
            report.duplicates,
            source_report.duplicates,
            "duplicate count",
        )?;
        report.reconciled = checked_add(
            report.reconciled,
            source_report.reconciled,
            "reconciled count",
        )?;
        report.quarantined = checked_add(
            report.quarantined,
            source_report.quarantined,
            "quarantine count",
        )?;
        if source_report.safe_error_code.is_some() {
            report.safe_error_code = source_report.safe_error_code;
        }
    }
    Ok(report)
}

pub fn scan_source_once(
    store: &Store,
    source: &UsageScanSource,
    limits: &UsageScannerLimits,
) -> Result<UsageScanRunReport, StoreError> {
    let deadline = Instant::now()
        .checked_add(limits.max_scan_duration)
        .unwrap_or_else(Instant::now);
    scan_source_until(store, source, limits, deadline)
}

fn scan_source_until(
    store: &Store,
    source: &UsageScanSource,
    limits: &UsageScannerLimits,
    deadline: Instant,
) -> Result<UsageScanRunReport, StoreError> {
    let started_at_ms = now_ms();
    let parser_version = source.parser_provider.parser_version();
    let mut report = UsageScanRunReport::default();
    let mut diagnostic = UsageScannerDiagnostic {
        environment_id: source.environment_id.clone(),
        provider_instance_id: source.provider_instance_id.clone(),
        source_kind: source.source_kind.clone(),
        coverage_state: UsageSourceCoverageState::Complete,
        parser_version,
        last_scan_started_at_ms: started_at_ms,
        last_success_at_ms: None,
        latest_source_at_ms: None,
        source_lag_ms: None,
        files_scanned: 0,
        bytes_scanned: 0,
        quarantined_count: 0,
        last_safe_error_code: None,
    };

    if let Some(error) = source.mapping_error.as_ref() {
        diagnostic.coverage_state = UsageSourceCoverageState::Unavailable;
        diagnostic.last_safe_error_code = Some(error.clone());
        store.record_usage_scan_batch(&source.environment_id, &[], None, &[], Some(&diagnostic))?;
        report.safe_error_code = Some(error.clone());
        return Ok(report);
    }

    let enumeration = enumerate_jsonl_files(&source.root, limits, deadline);
    let enumeration = match enumeration {
        Ok(enumeration) => enumeration,
        Err(error) => {
            let code = io_error_code(&error).to_string();
            diagnostic.coverage_state = UsageSourceCoverageState::Unavailable;
            diagnostic.last_safe_error_code = Some(code.clone());
            store.record_usage_scan_batch(
                &source.environment_id,
                &[],
                None,
                &[],
                Some(&diagnostic),
            )?;
            report.safe_error_code = Some(code);
            return Ok(report);
        }
    };
    if let Some(error) = enumeration.safe_error_code {
        diagnostic.coverage_state = UsageSourceCoverageState::Partial;
        diagnostic.last_safe_error_code = Some(error.clone());
        report.safe_error_code = Some(error);
    }

    let mut remaining_bytes = limits.max_bytes_per_scan;
    for path in enumeration.files {
        if Instant::now() >= deadline {
            mark_partial(&mut diagnostic, &mut report, "scan_time_limit");
            break;
        }
        if remaining_bytes == 0 {
            mark_partial(&mut diagnostic, &mut report, "scan_byte_limit");
            break;
        }
        match scan_file(store, source, &path, limits, remaining_bytes, deadline) {
            Ok(file_report) => {
                remaining_bytes = remaining_bytes.saturating_sub(file_report.bytes_scanned);
                report.files_scanned = checked_add(report.files_scanned, 1, "file count")?;
                report.bytes_scanned = checked_add(
                    report.bytes_scanned,
                    file_report.bytes_scanned,
                    "byte count",
                )?;
                report.inserted =
                    checked_add(report.inserted, file_report.inserted, "insert count")?;
                report.duplicates =
                    checked_add(report.duplicates, file_report.duplicates, "duplicate count")?;
                report.reconciled = checked_add(
                    report.reconciled,
                    file_report.reconciled,
                    "reconciled count",
                )?;
                report.quarantined = checked_add(
                    report.quarantined,
                    file_report.quarantined,
                    "quarantine count",
                )?;
                if let Some(source_time) = file_report.latest_source_at_ms {
                    diagnostic.latest_source_at_ms = Some(
                        diagnostic
                            .latest_source_at_ms
                            .map_or(source_time, |current| current.max(source_time)),
                    );
                }
                if file_report.partial {
                    diagnostic.coverage_state = UsageSourceCoverageState::Partial;
                }
                if let Some(error) = file_report.safe_error_code {
                    diagnostic.last_safe_error_code = Some(error.clone());
                    report.safe_error_code = Some(error);
                }
            }
            Err(error) => {
                mark_partial(&mut diagnostic, &mut report, store_error_code(&error));
            }
        }
    }

    diagnostic.files_scanned = report.files_scanned;
    diagnostic.bytes_scanned = report.bytes_scanned;
    diagnostic.last_success_at_ms = Some(now_ms());
    diagnostic.source_lag_ms = diagnostic
        .latest_source_at_ms
        .map(|source_time| now_ms().saturating_sub(source_time));
    store.record_usage_scan_batch(&source.environment_id, &[], None, &[], Some(&diagnostic))?;
    Ok(report)
}

struct EnumerationResult {
    files: Vec<PathBuf>,
    safe_error_code: Option<String>,
}

fn enumerate_jsonl_files(
    root: &Path,
    limits: &UsageScannerLimits,
    deadline: Instant,
) -> io::Result<EnumerationResult> {
    let root_metadata = fs::symlink_metadata(root)?;
    if is_link_or_reparse(&root_metadata) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "scanner root cannot be a link or reparse point",
        ));
    }
    if !root_metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "scanner root is not a directory",
        ));
    }

    let mut stack = vec![root.to_path_buf()];
    let mut files = Vec::new();
    let mut directories = 0_usize;
    let mut visited = HashSet::new();
    let mut safe_error_code = None;

    while let Some(directory) = stack.pop() {
        if Instant::now() >= deadline {
            safe_error_code = Some("scan_time_limit".to_string());
            break;
        }
        directories = directories.saturating_add(1);
        if directories > limits.max_directories {
            safe_error_code = Some("directory_limit".to_string());
            break;
        }
        if let Some(identity) = directory_identity(&directory)?
            && !visited.insert(identity)
        {
            safe_error_code = Some("directory_cycle_skipped".to_string());
            continue;
        }

        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) => {
                safe_error_code = Some(io_error_code(&error).to_string());
                continue;
            }
        };
        let mut paths = Vec::new();
        for entry in entries {
            match entry {
                Ok(entry) => paths.push(entry.path()),
                Err(error) => {
                    safe_error_code = Some(io_error_code(&error).to_string());
                }
            }
        }
        paths.sort();
        for path in paths.into_iter().rev() {
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) => {
                    safe_error_code = Some(io_error_code(&error).to_string());
                    continue;
                }
            };
            if is_link_or_reparse(&metadata) {
                safe_error_code = Some("link_skipped".to_string());
                continue;
            }
            if metadata.is_dir() {
                stack.push(path);
            } else if metadata.is_file()
                && path.extension().and_then(|extension| extension.to_str()) == Some("jsonl")
            {
                if files.len() >= limits.max_files {
                    safe_error_code = Some("file_limit".to_string());
                    break;
                }
                if metadata.len() > limits.max_file_bytes {
                    safe_error_code = Some("file_size_limit".to_string());
                    continue;
                }
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(EnumerationResult {
        files,
        safe_error_code,
    })
}

#[derive(Default)]
struct FileScanReport {
    bytes_scanned: u64,
    inserted: u64,
    duplicates: u64,
    reconciled: u64,
    quarantined: u64,
    latest_source_at_ms: Option<u64>,
    partial: bool,
    safe_error_code: Option<String>,
}

fn scan_file(
    store: &Store,
    source: &UsageScanSource,
    path: &Path,
    limits: &UsageScannerLimits,
    remaining_bytes: u64,
    deadline: Instant,
) -> Result<FileScanReport, StoreError> {
    let subscription_key = source.subscription_key.as_ref().ok_or_else(|| {
        StoreError::Data("usage scanner subscription mapping is unavailable".to_string())
    })?;
    let canonical_path = fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_string();
    let file = File::open(path).map_err(|error| scanner_io_error("file_open_failed", error))?;
    let metadata = file
        .metadata()
        .map_err(|error| scanner_io_error("file_metadata_failed", error))?;
    let identity = platform_file_identity(&metadata, &file)
        .map_err(|error| scanner_io_error("file_identity_failed", error))?;
    let identity_key = identity.stable_key();
    let node_store_id = store.usage_node_store_id()?;
    let existing = store.get_usage_scanner_cursor(
        &source.environment_id,
        &source.provider_instance_id,
        &source.source_kind,
        &identity_key,
    )?;
    let (start_offset, mut parser, reset_code) = restore_parser(
        existing.as_ref(),
        &file,
        metadata.len(),
        source.parser_provider,
        &node_store_id,
    )?;
    let mut reader = BufReader::new(file);
    reader
        .seek(SeekFrom::Start(start_offset))
        .map_err(|error| scanner_io_error("file_seek_failed", error))?;

    let mut offset = start_offset;
    let mut report = FileScanReport::default();
    if let Some(code) = reset_code {
        report.safe_error_code = Some(code);
    }
    let mut observations = Vec::new();
    let mut quarantines = Vec::new();
    let scan_budget = remaining_bytes.min(limits.max_file_bytes);
    let mut records = 0_usize;

    while report.bytes_scanned < scan_budget {
        if Instant::now() >= deadline {
            report.partial = true;
            report.safe_error_code = Some("scan_time_limit".to_string());
            break;
        }
        if records >= limits.max_records_per_file {
            report.partial = true;
            report.safe_error_code = Some("record_limit".to_string());
            break;
        }
        let record_offset = offset;
        let available = scan_budget.saturating_sub(report.bytes_scanned);
        match read_bounded_record(&mut reader, limits.max_record_bytes, available, deadline)
            .map_err(|error| scanner_io_error("file_read_failed", error))?
        {
            BoundedRecord::Eof => break,
            BoundedRecord::Incomplete => {
                report.partial = true;
                report.safe_error_code = Some("incomplete_trailing_record".to_string());
                break;
            }
            BoundedRecord::BudgetReached => {
                report.partial = true;
                report.safe_error_code = Some("scan_byte_limit".to_string());
                break;
            }
            BoundedRecord::Complete {
                bytes,
                consumed,
                oversized,
            } => {
                offset = offset
                    .checked_add(consumed)
                    .ok_or_else(|| StoreError::Data("scanner file offset overflow".to_string()))?;
                report.bytes_scanned =
                    checked_add(report.bytes_scanned, consumed, "scanner byte count")?;
                records = records.saturating_add(1);
                let content = trim_jsonl_ending(&bytes);
                if content.iter().all(u8::is_ascii_whitespace) {
                    continue;
                }
                if oversized {
                    push_quarantine(
                        &mut quarantines,
                        limits,
                        source,
                        &identity_key,
                        record_offset,
                        QuarantineReason {
                            code: "record_too_large",
                            detail: "complete JSONL record exceeded the configured size limit",
                        },
                        content,
                    );
                    report.partial = true;
                    report.safe_error_code = Some("record_too_large".to_string());
                    continue;
                }
                match parser.parse_complete_record(content) {
                    TranscriptRecordOutcome::Ignored => {}
                    TranscriptRecordOutcome::Malformed(error) => {
                        push_quarantine(
                            &mut quarantines,
                            limits,
                            source,
                            &identity_key,
                            record_offset,
                            QuarantineReason {
                                code: error.code,
                                detail: "complete JSONL record failed provider parsing",
                            },
                            content,
                        );
                        report.partial = true;
                        report.safe_error_code = Some(error.code.to_string());
                    }
                    TranscriptRecordOutcome::Usage(usage) => {
                        if usage.unknown_fields.iter().any(|field| {
                            !matches!(
                                field,
                                pandamux_core::UsageUnknownField::CacheReadTokens
                                    | pandamux_core::UsageUnknownField::CacheWriteTokens
                            )
                        }) {
                            report.partial = true;
                            report.safe_error_code = Some("usage_fields_unknown".to_string());
                        }
                        if let Some(occurred_at_ms) = usage.occurred_at_ms {
                            report.latest_source_at_ms = Some(
                                report
                                    .latest_source_at_ms
                                    .map_or(occurred_at_ms, |current| current.max(occurred_at_ms)),
                            );
                        }
                        observations.push(parsed_observation(
                            source,
                            subscription_key,
                            &identity_key,
                            record_offset,
                            content,
                            usage,
                        ));
                    }
                }
            }
        }
    }

    let mut file = reader.into_inner();
    let final_metadata = file
        .metadata()
        .map_err(|error| scanner_io_error("file_metadata_failed", error))?;
    if final_metadata.len() < offset {
        return Err(StoreError::Data(
            "usage transcript changed while it was being scanned".to_string(),
        ));
    }
    let prefix_hash = committed_prefix_hash(&mut file, offset)
        .map_err(|error| scanner_io_error("file_prefix_failed", error))?;
    let parser_state_json = serde_json::to_string(&parser)?;
    let cursor = UsageScannerCursor {
        environment_id: source.environment_id.clone(),
        provider_instance_id: source.provider_instance_id.clone(),
        source_kind: source.source_kind.clone(),
        node_store_id,
        file_identity: identity_key,
        canonical_path,
        committed_offset: offset,
        committed_prefix_sha256: prefix_hash,
        parser_version: parser.parser_version(),
        parser_state_json,
        observed_size: final_metadata.len(),
        observed_modified_at_ms: modified_at_ms(&final_metadata),
        updated_at_ms: now_ms(),
    };
    let result = store.record_usage_scan_batch(
        &source.environment_id,
        &observations,
        Some(&cursor),
        &quarantines,
        None,
    )?;
    report.inserted = result.inserted;
    report.duplicates = result.duplicates;
    report.reconciled = result.reconciled;
    report.quarantined = result.quarantined;
    if result.reconciliation_mismatches > 0 {
        report.partial = true;
        report.safe_error_code = Some("managed_transcript_mismatch".to_string());
    }
    Ok(report)
}

fn restore_parser(
    cursor: Option<&UsageScannerCursor>,
    file: &File,
    size: u64,
    provider: TranscriptProvider,
    node_store_id: &NodeStoreId,
) -> Result<(u64, TranscriptParser, Option<String>), StoreError> {
    let Some(cursor) = cursor else {
        return Ok((0, TranscriptParser::new(provider), None));
    };
    if &cursor.node_store_id != node_store_id {
        return Ok((
            0,
            TranscriptParser::new(provider),
            Some("node_store_changed".to_string()),
        ));
    }
    if cursor.parser_version != provider.parser_version() {
        return Ok((
            0,
            TranscriptParser::new(provider),
            Some("parser_version_changed".to_string()),
        ));
    }
    if size < cursor.committed_offset {
        return Ok((
            0,
            TranscriptParser::new(provider),
            Some("file_truncated".to_string()),
        ));
    }
    let mut file = file
        .try_clone()
        .map_err(|error| scanner_io_error("file_clone_failed", error))?;
    let current_prefix = committed_prefix_hash(&mut file, cursor.committed_offset)
        .map_err(|error| scanner_io_error("file_prefix_failed", error))?;
    if current_prefix != cursor.committed_prefix_sha256 {
        return Ok((
            0,
            TranscriptParser::new(provider),
            Some("file_replaced".to_string()),
        ));
    }
    let parser = match serde_json::from_str::<TranscriptParser>(&cursor.parser_state_json) {
        Ok(parser) if parser.provider() == provider => parser,
        _ => {
            return Ok((
                0,
                TranscriptParser::new(provider),
                Some("parser_state_invalid".to_string()),
            ));
        }
    };
    Ok((cursor.committed_offset, parser, None))
}

enum BoundedRecord {
    Eof,
    Incomplete,
    BudgetReached,
    Complete {
        bytes: Vec<u8>,
        consumed: u64,
        oversized: bool,
    },
}

fn read_bounded_record<R: BufRead>(
    reader: &mut R,
    max_record_bytes: usize,
    byte_budget: u64,
    deadline: Instant,
) -> io::Result<BoundedRecord> {
    if byte_budget == 0 {
        return Ok(BoundedRecord::BudgetReached);
    }
    let probe_limit = u64::try_from(max_record_bytes)
        .unwrap_or(u64::MAX)
        .saturating_add(2)
        .min(byte_budget);
    let mut bytes =
        Vec::with_capacity(usize::try_from(probe_limit.min(64 * 1024)).unwrap_or(64 * 1024));
    let consumed = reader.take(probe_limit).read_until(b'\n', &mut bytes)?;
    if consumed == 0 {
        return Ok(BoundedRecord::Eof);
    }
    let consumed = u64::try_from(consumed).unwrap_or(u64::MAX);
    if bytes.ends_with(b"\n") {
        let oversized = trim_jsonl_ending(&bytes).len() > max_record_bytes;
        return Ok(BoundedRecord::Complete {
            bytes,
            consumed,
            oversized,
        });
    }
    if consumed < probe_limit {
        return Ok(BoundedRecord::Incomplete);
    }
    if probe_limit == byte_budget {
        return Ok(BoundedRecord::BudgetReached);
    }

    let mut total = consumed;
    loop {
        if Instant::now() >= deadline {
            return Ok(BoundedRecord::BudgetReached);
        }
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            return Ok(BoundedRecord::Incomplete);
        }
        let remaining = byte_budget.saturating_sub(total);
        if remaining == 0 {
            return Ok(BoundedRecord::BudgetReached);
        }
        let available = usize::try_from(remaining.min(buffer.len() as u64)).unwrap_or(buffer.len());
        let slice = &buffer[..available];
        if let Some(index) = slice.iter().position(|byte| *byte == b'\n') {
            let advance = index + 1;
            reader.consume(advance);
            total = total.saturating_add(u64::try_from(advance).unwrap_or(u64::MAX));
            return Ok(BoundedRecord::Complete {
                bytes,
                consumed: total,
                oversized: true,
            });
        }
        reader.consume(available);
        total = total.saturating_add(u64::try_from(available).unwrap_or(u64::MAX));
    }
}

fn trim_jsonl_ending(bytes: &[u8]) -> &[u8] {
    bytes
        .strip_suffix(b"\n")
        .unwrap_or(bytes)
        .strip_suffix(b"\r")
        .unwrap_or_else(|| bytes.strip_suffix(b"\n").unwrap_or(bytes))
}

#[derive(Clone, Copy)]
struct QuarantineReason {
    code: &'static str,
    detail: &'static str,
}

fn push_quarantine(
    quarantines: &mut Vec<UsageQuarantineRecord>,
    limits: &UsageScannerLimits,
    source: &UsageScanSource,
    file_identity: &str,
    record_offset: u64,
    reason: QuarantineReason,
    safe_excerpt: &[u8],
) {
    if quarantines.len() >= limits.max_quarantines_per_file {
        return;
    }
    quarantines.push(UsageQuarantineRecord {
        environment_id: source.environment_id.clone(),
        provider_instance_id: source.provider_instance_id.clone(),
        source_kind: source.source_kind.clone(),
        file_identity_sha256: sha256_hex(file_identity.as_bytes()),
        record_offset,
        parser_version: source.parser_provider.parser_version(),
        reason_code: reason.code.to_string(),
        reason_detail: reason.detail.to_string(),
        safe_excerpt_sha256: sha256_hex(safe_excerpt),
        created_at_ms: now_ms(),
    });
}

fn parsed_observation(
    source: &UsageScanSource,
    subscription_key: &SubscriptionKey,
    file_identity: &str,
    record_offset: u64,
    record: &[u8],
    usage: ParsedTranscriptUsage,
) -> UsageObservation {
    let stable_identity = if let Some(turn_id) = usage.provider_native_turn_id.as_deref() {
        stable_hash_parts(&[
            source.provider.as_str(),
            source.provider_instance_id.as_str(),
            usage.provider_native_session_id.as_deref().unwrap_or(""),
            turn_id,
        ])
    } else if let (Some(session_id), Some(occurred_at_ms)) = (
        usage.provider_native_session_id.as_deref(),
        usage.occurred_at_ms,
    ) {
        let tokens = serde_json::to_string(&usage.tokens).unwrap_or_default();
        let unknown_fields = serde_json::to_string(&usage.unknown_fields).unwrap_or_default();
        stable_hash_parts(&[
            source.provider.as_str(),
            source.provider_instance_id.as_str(),
            session_id,
            &occurred_at_ms.to_string(),
            usage.model.as_deref().unwrap_or(""),
            &tokens,
            &unknown_fields,
        ])
    } else {
        stable_hash_parts(&[
            source.provider.as_str(),
            source.provider_instance_id.as_str(),
            file_identity,
            &record_offset.to_string(),
            &sha256_hex(record),
        ])
    };
    UsageObservation {
        id: UsageObservationId::new(format!("usage-transcript-{stable_identity}")),
        source_kind: source.source_kind.clone(),
        parser_version: source.parser_provider.parser_version(),
        provider_native_session_id: usage.provider_native_session_id,
        provider_native_turn_id: usage.provider_native_turn_id,
        occurred_at_ms: usage.occurred_at_ms,
        provider: source.provider.clone(),
        provider_instance_id: source.provider_instance_id.clone(),
        subscription_key: subscription_key.clone(),
        model: usage.model,
        tokens: usage.tokens,
        unknown_fields: usage.unknown_fields,
        provider_reported_micro_usd: usage.provider_reported_micro_usd,
        source_record_count: 1,
        thread_id: None,
        turn_id: None,
        agent_id: None,
        schedule_id: None,
        fact_kind: if source.parser_provider == TranscriptProvider::Claude {
            UsageFactKind::TokenComponent
        } else {
            UsageFactKind::Aggregate
        },
    }
}

fn committed_prefix_hash(file: &mut File, offset: u64) -> io::Result<String> {
    let start = offset.saturating_sub(PREFIX_WINDOW_BYTES);
    let length = usize::try_from(offset.saturating_sub(start)).unwrap_or(0);
    let mut bytes = vec![0_u8; length];
    file.seek(SeekFrom::Start(start))?;
    file.read_exact(&mut bytes)?;
    Ok(sha256_hex(&bytes))
}

fn modified_at_ms(metadata: &Metadata) -> Option<u64> {
    metadata
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
}

#[cfg(unix)]
fn platform_file_identity(metadata: &Metadata, _file: &File) -> io::Result<PlatformFileIdentity> {
    use std::os::unix::fs::MetadataExt;
    Ok(PlatformFileIdentity::Unix {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(windows)]
fn platform_file_identity(_metadata: &Metadata, file: &File) -> io::Result<PlatformFileIdentity> {
    use std::mem::zeroed;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };

    let mut information: BY_HANDLE_FILE_INFORMATION = unsafe { zeroed() };
    let result =
        unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), &mut information) };
    if result == 0 {
        return Err(io::Error::last_os_error());
    }
    let file_index =
        (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow);
    Ok(PlatformFileIdentity::Windows {
        volume_serial: information.dwVolumeSerialNumber,
        file_index,
    })
}

#[cfg(not(any(unix, windows)))]
fn platform_file_identity(_metadata: &Metadata, _file: &File) -> io::Result<PlatformFileIdentity> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "platform file identity is unavailable",
    ))
}

#[cfg(unix)]
fn directory_identity(path: &Path) -> io::Result<Option<String>> {
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::metadata(path)?;
    Ok(Some(format!("{}:{}", metadata.dev(), metadata.ino())))
}

#[cfg(not(unix))]
fn directory_identity(_path: &Path) -> io::Result<Option<String>> {
    Ok(None)
}

#[cfg(windows)]
fn is_link_or_reparse(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_link_or_reparse(metadata: &Metadata) -> bool {
    metadata.file_type().is_symlink()
}

fn mark_partial(
    diagnostic: &mut UsageScannerDiagnostic,
    report: &mut UsageScanRunReport,
    code: &str,
) {
    diagnostic.coverage_state = UsageSourceCoverageState::Partial;
    diagnostic.last_safe_error_code = Some(code.to_string());
    report.safe_error_code = Some(code.to_string());
}

fn scanner_io_error(code: &str, error: io::Error) -> StoreError {
    let code = match error.kind() {
        io::ErrorKind::NotFound => "source_unavailable",
        io::ErrorKind::PermissionDenied => "source_permission_denied",
        _ => code,
    };
    StoreError::Data(format!("{code}: {}", error.kind()))
}

fn store_error_code(error: &StoreError) -> &str {
    match error {
        StoreError::Sql(_) => "scanner_store_sql_failed",
        StoreError::Json(_) => "scanner_store_json_failed",
        StoreError::Io(_) => "scanner_store_io_failed",
        StoreError::RevisionDrift { .. } => "scanner_store_revision_drift",
        StoreError::Data(message) => message
            .split_once(':')
            .map(|(code, _)| code)
            .filter(|code| {
                !code.is_empty()
                    && code.len() <= 64
                    && code.bytes().all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'
                    })
            })
            .unwrap_or("scanner_store_data_failed"),
    }
}

fn io_error_code(error: &io::Error) -> &'static str {
    match error.kind() {
        io::ErrorKind::NotFound => "source_unavailable",
        io::ErrorKind::PermissionDenied => "source_permission_denied",
        _ => "source_read_failed",
    }
}

fn checked_add(left: u64, right: u64, field: &str) -> Result<u64, StoreError> {
    left.checked_add(right)
        .ok_or_else(|| StoreError::Data(format!("usage scanner {field} overflow")))
}

fn stable_hash_parts(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_le_bytes());
        hasher.update(part.as_bytes());
    }
    digest_hex(&hasher.finalize())
}

fn sha256_hex(bytes: &[u8]) -> String {
    digest_hex(&Sha256::digest(bytes))
}

fn digest_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandamux_core::{UsageSubscription, UserSettings};
    use pandamux_protocol::{UsageQueryParams, UsageSourceCoverageState};
    use std::fs::OpenOptions;
    use std::io::Write;
    use tempfile::{NamedTempFile, TempDir};

    fn test_limits() -> UsageScannerLimits {
        UsageScannerLimits {
            max_directories: 16,
            max_files: 16,
            max_file_bytes: 1024 * 1024,
            max_bytes_per_scan: 1024 * 1024,
            max_record_bytes: 512,
            max_records_per_file: 128,
            max_managed_turns_per_scan: 128,
            max_quarantines_per_file: 16,
            max_scan_duration: Duration::from_secs(2),
        }
    }

    fn configure_store() -> Store {
        let store = Store::in_memory().expect("open scanner store");
        store
            .save_usage_subscription(
                &UsageSubscription {
                    key: SubscriptionKey::new("subkey-scanner"),
                    provider: UsageProvider::Claude,
                    display_name: "Scanner fixture".to_string(),
                    masked_account_label: None,
                },
                1,
            )
            .expect("save scanner subscription");
        store
            .map_provider_subscription(
                &EnvironmentId::new("env-scanner"),
                &ProviderInstanceId::new("claude-scanner"),
                &SubscriptionKey::new("subkey-scanner"),
                1,
            )
            .expect("map scanner subscription");
        store
    }

    fn source(root: &Path) -> UsageScanSource {
        UsageScanSource {
            environment_id: EnvironmentId::new("env-scanner"),
            provider_instance_id: ProviderInstanceId::new("claude-scanner"),
            provider: UsageProvider::Claude,
            subscription_key: Some(SubscriptionKey::new("subkey-scanner")),
            source_kind: UsageSourceKind::ClaudeTranscript,
            parser_provider: TranscriptProvider::Claude,
            root: root.to_path_buf(),
            mapping_error: None,
        }
    }

    fn claude_record(id: &str, second: u8, input: u64) -> String {
        format!(
            "{{\"type\":\"assistant\",\"sessionId\":\"session-fixture\",\"timestamp\":\"2026-09-29T10:00:{second:02}Z\",\"message\":{{\"id\":\"{id}\",\"model\":\"claude-fixture\",\"usage\":{{\"input_tokens\":{input},\"output_tokens\":2}}}}}}"
        )
    }

    fn append(path: &Path, value: &str) {
        let mut file = OpenOptions::new()
            .append(true)
            .open(path)
            .expect("open transcript for append");
        file.write_all(value.as_bytes()).expect("append transcript");
        file.flush().expect("flush transcript");
    }

    #[test]
    fn node_store_identity_change_restarts_parser_from_zero() {
        let mut transcript = NamedTempFile::new().expect("temporary transcript");
        transcript
            .write_all(b"complete record\n")
            .expect("write transcript fixture");
        transcript.flush().expect("flush transcript fixture");
        let parser = TranscriptParser::new(TranscriptProvider::Claude);
        let cursor = UsageScannerCursor {
            environment_id: EnvironmentId::new("env-scanner"),
            provider_instance_id: ProviderInstanceId::new("claude-scanner"),
            source_kind: UsageSourceKind::ClaudeTranscript,
            node_store_id: NodeStoreId::new("store-before-reset"),
            file_identity: "unix:1:2".to_string(),
            canonical_path: transcript.path().to_string_lossy().to_string(),
            committed_offset: 8,
            committed_prefix_sha256: "0".repeat(64),
            parser_version: parser.parser_version(),
            parser_state_json: serde_json::to_string(&parser).expect("serialize parser"),
            observed_size: 16,
            observed_modified_at_ms: None,
            updated_at_ms: 1,
        };

        let (offset, restored, code) = restore_parser(
            Some(&cursor),
            transcript.as_file(),
            16,
            TranscriptProvider::Claude,
            &NodeStoreId::new("store-after-reset"),
        )
        .expect("restore after node store reset");
        assert_eq!(offset, 0);
        assert_eq!(restored, TranscriptParser::new(TranscriptProvider::Claude));
        assert_eq!(code.as_deref(), Some("node_store_changed"));
    }

    fn observation_count(store: &Store) -> usize {
        store
            .query_usage(&UsageQueryParams::default())
            .expect("query usage")
            .items
            .len()
    }

    #[test]
    fn scanner_handles_first_restart_append_and_partial_tail() {
        let temp = TempDir::new().expect("temporary transcript root");
        let path = temp.path().join("session.jsonl");
        fs::write(&path, format!("{}\n", claude_record("msg-1", 1, 10)))
            .expect("write first record");
        let store = configure_store();
        let source = source(temp.path());

        let first = scan_source_once(&store, &source, &test_limits()).expect("first scan");
        assert_eq!(first.inserted, 1);
        assert_eq!(observation_count(&store), 1);

        let restart = scan_source_once(&store, &source, &test_limits()).expect("restart scan");
        assert_eq!(restart.inserted, 0);
        assert_eq!(restart.bytes_scanned, 0);

        let second = claude_record("msg-2", 2, 20);
        append(&path, &second);
        let partial = scan_source_once(&store, &source, &test_limits()).expect("partial scan");
        assert_eq!(partial.inserted, 0);
        assert_eq!(
            partial.safe_error_code.as_deref(),
            Some("incomplete_trailing_record")
        );
        assert_eq!(observation_count(&store), 1);

        append(&path, "\n");
        let completed =
            scan_source_once(&store, &source, &test_limits()).expect("completed append scan");
        assert_eq!(completed.inserted, 1);
        assert_eq!(observation_count(&store), 2);
    }

    #[test]
    fn scanner_detects_rename_rotation_truncation_and_replacement() {
        let temp = TempDir::new().expect("temporary transcript root");
        let active = temp.path().join("active.jsonl");
        fs::write(&active, format!("{}\n", claude_record("msg-1", 1, 10)))
            .expect("write initial transcript");
        let store = configure_store();
        let source = source(temp.path());
        assert_eq!(
            scan_source_once(&store, &source, &test_limits())
                .expect("initial scan")
                .inserted,
            1
        );

        let rotated = temp.path().join("rotated.jsonl");
        fs::rename(&active, &rotated).expect("rename transcript");
        let renamed = scan_source_once(&store, &source, &test_limits()).expect("rename scan");
        assert_eq!(renamed.inserted, 0);
        assert_eq!(renamed.bytes_scanned, 0);

        fs::write(&active, format!("{}\n", claude_record("msg-2", 2, 20)))
            .expect("write rotated replacement");
        let rotation = scan_source_once(&store, &source, &test_limits()).expect("rotation scan");
        assert_eq!(rotation.inserted, 1);

        let mut same_file = OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&active)
            .expect("truncate active transcript");
        writeln!(same_file, "{}", claude_record("msg-3", 3, 30))
            .expect("write truncated transcript");
        same_file.flush().expect("flush truncated transcript");
        let truncation =
            scan_source_once(&store, &source, &test_limits()).expect("truncation scan");
        assert_eq!(truncation.inserted, 1);
        assert!(matches!(
            truncation.safe_error_code.as_deref(),
            Some("file_truncated" | "file_replaced")
        ));

        fs::remove_file(&active).expect("remove active transcript");
        fs::write(&active, format!("{}\n", claude_record("msg-4", 4, 40)))
            .expect("write replacement transcript");
        let replacement =
            scan_source_once(&store, &source, &test_limits()).expect("replacement scan");
        assert_eq!(replacement.inserted, 1);
        assert_eq!(observation_count(&store), 4);
    }

    #[test]
    fn scanner_resynchronizes_after_duplicate_malformed_and_oversized_records() {
        let temp = TempDir::new().expect("temporary transcript root");
        let path = temp.path().join("mixed.jsonl");
        let first = claude_record("msg-1", 1, 10);
        let repeated_chunk = claude_record("msg-1", 9, 10);
        let second = claude_record("msg-2", 2, 20);
        let oversized = format!("{{\"padding\":\"{}\"}}", "x".repeat(600));
        fs::write(
            &path,
            format!("{first}\n{repeated_chunk}\n{{bad json}}\n{oversized}\n{second}\n"),
        )
        .expect("write mixed transcript");
        let store = configure_store();
        let source = source(temp.path());

        let report = scan_source_once(&store, &source, &test_limits()).expect("mixed scan");
        assert_eq!(report.inserted, 2);
        assert_eq!(report.duplicates, 1);
        assert_eq!(report.quarantined, 2);
        assert_eq!(observation_count(&store), 2);
        let query = store
            .query_usage(&UsageQueryParams::default())
            .expect("query mixed usage");
        assert_eq!(query.coverage.quarantined_count, 2);
        assert!(
            query
                .items
                .iter()
                .all(|item| item.observation.fact_kind == UsageFactKind::TokenComponent)
        );
        assert_eq!(query.coverage.scanner_diagnostics.len(), 1);
        assert_eq!(
            query.coverage.scanner_diagnostics[0].coverage_state,
            UsageSourceCoverageState::Partial
        );
    }

    #[cfg(unix)]
    #[test]
    fn scanner_skips_symlink_cycle_without_following_it() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().expect("temporary transcript root");
        fs::write(
            temp.path().join("session.jsonl"),
            format!("{}\n", claude_record("msg-1", 1, 10)),
        )
        .expect("write transcript");
        symlink(temp.path(), temp.path().join("cycle")).expect("create cycle link");
        let store = configure_store();
        let report = scan_source_once(&store, &source(temp.path()), &test_limits())
            .expect("scan link cycle");
        assert_eq!(report.inserted, 1);
        assert_eq!(report.safe_error_code.as_deref(), Some("link_skipped"));
    }

    #[test]
    fn source_disappearance_is_durable_unavailable_diagnostic() {
        let temp = TempDir::new().expect("temporary transcript root");
        let missing = temp.path().join("missing");
        let store = configure_store();
        let report = scan_source_once(&store, &source(&missing), &test_limits())
            .expect("scan missing source");
        assert_eq!(
            report.safe_error_code.as_deref(),
            Some("source_unavailable")
        );
        let query = store
            .query_usage(&UsageQueryParams::default())
            .expect("query diagnostics");
        assert_eq!(query.coverage.scanner_diagnostics.len(), 1);
        assert_eq!(
            query.coverage.scanner_diagnostics[0].coverage_state,
            UsageSourceCoverageState::Unavailable
        );
        assert_eq!(
            query.coverage.scanner_diagnostics[0]
                .last_safe_error_code
                .as_deref(),
            Some("source_unavailable")
        );
    }

    #[test]
    fn platform_and_permission_fixtures_cover_windows_and_unix_abstractions() {
        assert_eq!(
            PlatformFileIdentity::Unix {
                device: 42,
                inode: 99
            }
            .stable_key(),
            "unix:42:99"
        );
        assert_eq!(
            PlatformFileIdentity::Windows {
                volume_serial: 7,
                file_index: 123
            }
            .stable_key(),
            "windows:7:123"
        );
        assert_eq!(
            io_error_code(&io::Error::new(
                io::ErrorKind::PermissionDenied,
                "fixture denial"
            )),
            "source_permission_denied"
        );
    }

    #[test]
    fn bounded_reader_never_commits_an_incomplete_record() {
        let mut reader = BufReader::new(io::Cursor::new(br#"{"incomplete":true"#.to_vec()));
        let result = read_bounded_record(
            &mut reader,
            128,
            128,
            Instant::now() + Duration::from_secs(1),
        )
        .expect("read fixture");
        assert!(matches!(result, BoundedRecord::Incomplete));
    }

    #[tokio::test]
    async fn service_shutdown_joins_worker_within_owned_drain_budget() {
        let temp = TempDir::new().expect("temporary data directory");
        let profile = temp.path().join("claude-profile");
        fs::create_dir_all(profile.join("projects")).expect("create transcript root");
        let store = Store::in_memory().expect("open service store");
        let mut settings = UserSettings::default();
        settings
            .providers
            .retain(|provider| provider.provider == ProviderKind::Claude);
        settings.providers[0].id = ProviderInstanceId::new("claude-scanner");
        settings.providers[0].profile_dir = profile.to_string_lossy().to_string();
        settings.environments[0].id = EnvironmentId::new("env-scanner");
        store
            .save_settings(&settings)
            .expect("save scanner settings");
        store
            .save_usage_subscription(
                &UsageSubscription {
                    key: SubscriptionKey::new("subkey-scanner"),
                    provider: UsageProvider::Claude,
                    display_name: "Scanner fixture".to_string(),
                    masked_account_label: None,
                },
                1,
            )
            .expect("save scanner subscription");
        store
            .map_provider_subscription(
                &EnvironmentId::new("env-scanner"),
                &ProviderInstanceId::new("claude-scanner"),
                &SubscriptionKey::new("subkey-scanner"),
                1,
            )
            .expect("map scanner subscription");
        let config = UsageScannerRuntimeConfig {
            limits: test_limits(),
            periodic_interval: Duration::from_secs(60),
            periodic_jitter: Duration::ZERO,
            trigger_ttl: Duration::from_secs(5),
            trigger_queue_capacity: 2,
            shutdown_poll_interval: Duration::from_millis(10),
        };
        let service = UsageScannerService::start(
            store,
            EnvironmentId::new("env-scanner"),
            temp.path().to_path_buf(),
            config,
        )
        .expect("start scanner service");
        let drain_budget = service.drain_budget();
        let started = Instant::now();
        service.shutdown().await.expect("join scanner service");
        assert!(started.elapsed() < drain_budget);
    }
}
