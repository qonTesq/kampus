//! The append-only JSONL audit file and its writer thread.
//!
//! Callers enqueue a serialized row on a bounded queue and wait for its
//! acknowledgement. One writer thread takes every queued row, writes them
//! together, syncs once and acknowledges each: durability per row, one
//! `fsync` per batch. A saturated queue makes callers wait a bounded time,
//! then fail closed.

use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use tokio::sync::{mpsc, oneshot};

use crate::contracts::{AuditError, AuditEvent, AuditSink, BoxFuture};

/// Rows waiting for the writer before callers feel backpressure.
const QUEUE_CAPACITY: usize = 1024;
/// Most rows committed by one `fsync`.
const BATCH_MAX: usize = 256;
/// How long a caller waits for queue space before failing closed.
const ENQUEUE_WAIT: Duration = Duration::from_secs(1);

struct Job {
    line: Vec<u8>,
    done: oneshot::Sender<Result<(), AuditError>>,
}

/// The file shared by the writer thread, pruning and reads.
#[derive(Debug)]
struct LogFile {
    path: PathBuf,
    retention: TimeDelta,
    /// Serializes appends, pruning and reads of the file.
    lock: Mutex<()>,
    syncs: AtomicUsize,
}

/// Append-only JSONL file with bounded retention.
#[derive(Debug)]
pub struct JsonlAudit {
    file: Arc<LogFile>,
    queue: mpsc::Sender<Job>,
}

impl std::fmt::Debug for Job {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Job").finish_non_exhaustive()
    }
}

impl JsonlAudit {
    /// Open (creating directories and the file) with `retention_hours`, and
    /// start the writer thread. It stops when this value is dropped.
    pub fn open(path: impl Into<PathBuf>, retention_hours: u32) -> Result<Self, AuditError> {
        Self::open_with_queue(path.into(), retention_hours, QUEUE_CAPACITY)
    }

    fn open_with_queue(
        path: PathBuf,
        retention_hours: u32,
        capacity: usize,
    ) -> Result<Self, AuditError> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            fs::create_dir_all(dir).map_err(|_| AuditError)?;
        }
        append_handle(&path)?;
        let file = Arc::new(LogFile {
            path,
            retention: TimeDelta::hours(i64::from(retention_hours)),
            lock: Mutex::new(()),
            syncs: AtomicUsize::new(0),
        });
        let (queue, jobs) = mpsc::channel(capacity);
        let writer = Arc::clone(&file);
        std::thread::Builder::new()
            .name("kampus-audit".to_owned())
            .spawn(move || writer.run(jobs))
            .map_err(|_| AuditError)?;
        Ok(Self { file, queue })
    }

    pub fn path(&self) -> &Path {
        &self.file.path
    }

    /// Drop rows older than the retention window (atomic rewrite, streamed).
    /// Returns how many rows were removed. Unparseable rows are dropped too.
    pub fn prune(&self, now: DateTime<Utc>) -> Result<usize, AuditError> {
        let cutoff = now - self.file.retention;
        let keep = |line: &str| {
            serde_json::from_str::<AuditEvent>(line).is_ok_and(|e| e.timestamp >= cutoff)
        };
        let _guard = self.file.lock();
        let mut removed = 0;
        self.file.for_each_row(|line| {
            if !keep(line) {
                removed += 1;
            }
            Ok(())
        })?;
        if removed == 0 {
            return Ok(0);
        }
        let tmp = self.file.path.with_extension("jsonl.tmp");
        let mut out = BufWriter::new(File::create(&tmp).map_err(|_| AuditError)?);
        self.file.for_each_row(|line| {
            if keep(line) {
                out.write_all(line.as_bytes()).map_err(|_| AuditError)?;
                out.write_all(b"\n").map_err(|_| AuditError)?;
            }
            Ok(())
        })?;
        let file = out.into_inner().map_err(|_| AuditError)?;
        file.sync_all().map_err(|_| AuditError)?;
        drop(file);
        fs::rename(&tmp, &self.file.path).map_err(|_| AuditError)?;
        Ok(removed)
    }

    /// The newest `limit` rows, newest first (for the local admin page).
    /// Holds at most `limit` rows in memory.
    pub fn recent(&self, limit: usize) -> Result<Vec<AuditEvent>, AuditError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let _guard = self.file.lock();
        let mut window = VecDeque::with_capacity(limit.min(1024));
        self.file.for_each_row(|line| {
            if let Ok(event) = serde_json::from_str::<AuditEvent>(line) {
                if window.len() == limit {
                    window.pop_front();
                }
                window.push_back(event);
            }
            Ok(())
        })?;
        Ok(window.into_iter().rev().collect())
    }
}

impl LogFile {
    fn lock(&self) -> std::sync::MutexGuard<'_, ()> {
        self.lock.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Writer thread: one blocking wait, then everything already queued.
    fn run(&self, mut jobs: mpsc::Receiver<Job>) {
        while let Some(first) = jobs.blocking_recv() {
            let mut batch = vec![first];
            while batch.len() < BATCH_MAX {
                match jobs.try_recv() {
                    Ok(job) => batch.push(job),
                    Err(_) => break,
                }
            }
            let result = self.append(&batch);
            if result.is_err() {
                // Operational signal only: no event fields are logged.
                tracing::error!("audit append failed; withholding data");
            }
            for job in batch {
                // The caller may have stopped waiting; nothing to do then.
                let _ = job.done.send(result.clone());
            }
        }
    }

    fn append(&self, batch: &[Job]) -> Result<(), AuditError> {
        let bytes: Vec<u8> = batch
            .iter()
            .flat_map(|job| job.line.iter().copied())
            .collect();
        let _guard = self.lock();
        let mut file = append_handle(&self.path)?;
        file.write_all(&bytes).map_err(|_| AuditError)?;
        file.flush().map_err(|_| AuditError)?;
        file.sync_data().map_err(|_| AuditError)?;
        self.syncs.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    /// Stream non-empty lines. The caller holds the lock.
    fn for_each_row(
        &self,
        mut row: impl FnMut(&str) -> Result<(), AuditError>,
    ) -> Result<(), AuditError> {
        let file = File::open(&self.path).map_err(|_| AuditError)?;
        for line in BufReader::new(file).lines() {
            let line = line.map_err(|_| AuditError)?;
            if !line.trim().is_empty() {
                row(&line)?;
            }
        }
        Ok(())
    }
}

fn append_handle(path: &Path) -> Result<File, AuditError> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|_| AuditError)
}

impl AuditSink for JsonlAudit {
    fn record<'a>(&'a self, event: &'a AuditEvent) -> BoxFuture<'a, Result<(), AuditError>> {
        Box::pin(async move {
            let mut line = serde_json::to_vec(event).map_err(|_| AuditError)?;
            line.push(b'\n');
            let (done, acknowledged) = oneshot::channel();
            let Ok(Ok(slot)) = tokio::time::timeout(ENQUEUE_WAIT, self.queue.reserve()).await
            else {
                tracing::error!("audit queue saturated; withholding data");
                return Err(AuditError);
            };
            slot.send(Job { line, done });
            acknowledged.await.unwrap_or(Err(AuditError))
        })
    }
}

#[cfg(test)]
impl JsonlAudit {
    /// A log with a small queue, for backpressure tests.
    pub(super) fn open_small(path: PathBuf, capacity: usize) -> Result<Self, AuditError> {
        Self::open_with_queue(path, 24, capacity)
    }

    /// Batches committed so far.
    pub(super) fn syncs(&self) -> usize {
        self.file.syncs.load(Ordering::SeqCst)
    }

    /// Hold the file lock (stalls the writer) until the guard drops.
    pub(super) fn hold(&self) -> std::sync::MutexGuard<'_, ()> {
        self.file.lock()
    }
}
