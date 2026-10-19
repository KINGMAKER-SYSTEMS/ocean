//! Room maintenance: the two sweeps that stop `rooms.db` and the attachment
//! blob tree from growing without bound.
//!
//! Durable rooms have always been append-only in practice. A transcript row is
//! written and never removed; an attachment's bytes are written, fsynced, and
//! removed only when somebody explicitly deletes the row that names them.
//! `room_attachments.rs` has said since it was written that a crash between the
//! blob write and the row commit "leaves it for a future GC sweep" — this is
//! that sweep, and the retention half beside it.
//!
//! Two jobs, one loop:
//!
//! 1. **Transcript retention.** A room CLOSED longer than the operator's window
//!    loses its transcript, its attachment rows, its read cursors and
//!    its federation dedup index in one IMMEDIATE store transaction per room;
//!    captured blob cleanup follows the row commit.
//!    Never an open room, at any age: the window is measured from the close, so
//!    a live room is not eligible however long it has been running. Off unless
//!    the operator turns it on — see [`DEFAULT_ROOM_RETENTION_DAYS`].
//! 2. **Attachment orphan GC.** Bytes on disk that no `room_attachments` row
//!    claims. This is the residue the upload path's deliberate write-then-commit
//!    order produces, plus whatever a retention cut left behind if the daemon
//!    died between the commit and the unlink.
//!
//! They share one loop because they share one shape — walk the store, touch the
//! filesystem, report — and because two independent timers over the same lock
//! and the same directory tree buy nothing but a way for them to interleave.
//!
//! **The report is the point of the operator half.** A sweep nobody can see is
//! indistinguishable from a sweep that is not running, and the failure this
//! module is most likely to have (a window set to the wrong unit, a blob root
//! that moved, a permissions error on one room's directory) is silent by
//! nature: disk simply keeps growing. So every run writes one `tracing::info!`
//! line and updates the `room_maintenance` card on `GET /health`, and the card
//! carries the CONFIGURATION as well as the counts — an operator reading
//! "retention_days: 0" learns why nothing was cut without going to the process
//! environment to find out.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::{extract::State, http::HeaderMap, http::StatusCode, Json};
use chrono::{DateTime, Utc};
use serde_json::json;

use crate::persistent_rooms::{with_rooms_handle, RoomStoreHandle};
use crate::AppState;

/// Environment variable naming the transcript retention window, in days.
pub(super) const RETENTION_DAYS_ENV: &str = "OCEAN_ROOM_RETENTION_DAYS";

/// The retention window a daemon that names none of its own runs with: **zero,
/// and zero means never**.
///
/// This is the one default in this module chosen for what it must not do rather
/// than for what it does. A transcript cut is unrecoverable — the rows are gone,
/// there is no tombstone holding the bodies, and the blobs are unlinked — and
/// the population that would inherit a nonzero default is every daemon that
/// upgrades into this code with rooms already in its store. A daemon that starts
/// deleting a member's history because it was restarted is a data-loss incident,
/// not a feature landing, and no window short enough to be useful is also short
/// enough to be safe to impose on somebody who never asked for it.
///
/// So retention is opt-in: unset or `0` keeps every closed room forever, exactly
/// as today. The number an operator who wants it should reach for is in the
/// operator guide beside `OCEAN_DB_PATH`, where it can be argued in prose rather
/// than imposed by a constant.
pub(super) const DEFAULT_ROOM_RETENTION_DAYS: u32 = 0;

/// How often the maintenance loop sweeps.
///
/// Six hours. Each pass enumerates the current blob tree; the interval bounds
/// cleanup lag without placing that walk on every upload's hot path.
pub(super) const ROOM_MAINTENANCE_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

/// How new a blob has to be for the orphan sweep to leave it alone.
///
/// One hour of recovery grace. A fixed sweep-start cutoff prevents aging-in;
/// publication custody and live reference checks protect in-flight writes even
/// when filesystem or store work stalls longer than this interval.
pub(super) const ATTACHMENT_ORPHAN_GRACE: Duration = Duration::from_secs(60 * 60);

/// Read the retention window once, at startup.
///
/// Once because the whole daemon reads its environment once: a value re-read per
/// sweep could change under a running loop, and a retention window that changes
/// without a restart is a window nobody can reason about from `/health`.
///
/// `0`, unset, empty, and anything that does not parse as a non-negative integer
/// all mean **never**. Refusing to guess is the point — a typo in a variable
/// whose job is deleting transcripts must not become a shorter window than the
/// operator wrote, and the one safe direction to fail is "keep everything". A
/// value that was present and unusable is logged at `warn`, because silence
/// there would let an operator believe retention was on.
pub(super) fn retention_days_from_env() -> u32 {
    match std::env::var(RETENTION_DAYS_ENV) {
        Err(_) => DEFAULT_ROOM_RETENTION_DAYS,
        Ok(raw) => parse_retention_days(&raw),
    }
}

/// The parse [`retention_days_from_env`] applies, split out so it is testable
/// without writing process environment.
fn parse_retention_days(raw: &str) -> u32 {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return DEFAULT_ROOM_RETENTION_DAYS;
    }
    match trimmed.parse::<u32>() {
        Ok(days) => days,
        Err(_) => {
            tracing::warn!(
                var = RETENTION_DAYS_ENV,
                "room retention window is not a non-negative integer number of days; \
                 retention stays OFF rather than guessing a shorter window"
            );
            DEFAULT_ROOM_RETENTION_DAYS
        }
    }
}

/// Everything one sweep needs to know, resolved at startup and never re-read.
#[derive(Debug, Clone, Copy)]
pub(super) struct MaintenanceConfig {
    /// Days a room may stay closed before its transcript is cut. `0` = never.
    pub(super) retention_days: u32,
    /// How new a blob has to be to survive the orphan sweep.
    pub(super) orphan_grace: Duration,
    /// How often the loop runs.
    pub(super) interval: Duration,
}

impl Default for MaintenanceConfig {
    /// The policy with retention OFF and the module's own grace and interval.
    ///
    /// This is what every test daemon gets, and it is the right thing for one to
    /// get: a fixture that inherited a live retention window would delete its
    /// own closed rooms out from under whatever it was actually asserting. The
    /// retention tests build their window explicitly and drive
    /// [`run_sweep`] directly with a fixed clock.
    fn default() -> Self {
        Self {
            retention_days: DEFAULT_ROOM_RETENTION_DAYS,
            orphan_grace: ATTACHMENT_ORPHAN_GRACE,
            interval: ROOM_MAINTENANCE_INTERVAL,
        }
    }
}

impl MaintenanceConfig {
    pub(super) fn from_env() -> Self {
        Self {
            retention_days: retention_days_from_env(),
            ..Self::default()
        }
    }
}

/// What one sweep did, plus the configuration it did it under.
///
/// Serialized wholesale as the `room_maintenance` object on `GET /health`. Every
/// field is a count, a duration, or a fixed error string — no room key, no
/// filename, no transcript body — because this card is scraped and logged, and
/// a maintenance report is not a place to publish the content it just deleted.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub(super) struct RoomMaintenanceReport {
    /// Sweep interval in seconds. Fixed at startup.
    pub(super) interval_secs: u64,
    /// Configured retention window in days. `0` means retention is off, which
    /// is why this is reported even though it never changes: a card showing
    /// zero rooms cut is ambiguous until you can see whether cutting is on.
    pub(super) retention_days: u32,
    /// Orphan grace window in seconds. Fixed at startup.
    pub(super) orphan_grace_secs: u64,
    /// When the last sweep finished. `None` before the first one.
    pub(super) last_run_at: Option<String>,
    /// How long the last sweep took.
    pub(super) last_run_ms: u64,
    /// Sweeps completed since daemon start, successful or not.
    pub(super) runs_total: u64,
    /// Rooms whose transcript was cut by the last sweep.
    pub(super) rooms_cut: u64,
    /// Transcript rows the last sweep removed.
    pub(super) messages_removed: u64,
    /// `room_attachments` rows the last sweep removed.
    pub(super) attachment_rows_removed: u64,
    /// Read-cursor rows (local + mirrored) the last sweep removed.
    pub(super) cursors_removed: u64,
    /// `federated_events` index rows the last sweep removed.
    pub(super) federated_index_rows_removed: u64,
    /// Unreferenced blob FILES the last sweep unlinked.
    pub(super) orphan_files_removed: u64,
    /// Whole room DIRECTORIES matching no room the store knows, removed by the
    /// last sweep.
    pub(super) orphan_dirs_removed: u64,
    /// Blob unlinks that FAILED in the last sweep, across both jobs.
    ///
    /// Its own counter rather than only an `last_error` string, because this is
    /// the one failure here that is invisible by construction: the rows commit,
    /// every count looks healthy, and disk simply never comes back. A nonzero
    /// value with `bytes_reclaimed` short of what was cut is the signature of a
    /// blob tree the daemon can read but not write.
    pub(super) blobs_unlink_failed: u64,
    /// Bytes the last sweep ACTUALLY reclaimed — a blob counts only once its
    /// file is gone, never merely because its row was deleted.
    pub(super) bytes_reclaimed: u64,
    /// The last sweep's error, or `None` if it was clean. A fixed, bounded
    /// string — never a path, a room key, or a store message that could carry
    /// one. It is deliberately NOT cleared by a later clean run's absence of
    /// errors being reported elsewhere: a clean sweep sets it to `None`, so a
    /// non-null value always describes the most recent sweep.
    pub(super) last_error: Option<String>,
    /// Current stage while running, otherwise the last completion/failure stage.
    pub(super) stage: MaintenanceStage,
    /// Classification and accounting describe the last completed attempt.
    pub(super) classification: MaintenanceClassification,
    pub(super) accounting: MaintenanceAccounting,
    /// Set before queueing blocking work; a stalled worker remains visible and
    /// retains its permit. Counts remain the last completed attempt's counts.
    pub(super) in_progress_since: Option<String>,
    /// When the scheduler will start its next sweep. Republished after every
    /// scheduled iteration, including one that panicked, so a value in the
    /// past means the loop itself has stopped rather than a sweep failing.
    pub(super) next_due_at: Option<String>,
}

impl RoomMaintenanceReport {
    fn new(config: &MaintenanceConfig) -> Self {
        Self {
            interval_secs: config.interval.as_secs(),
            retention_days: config.retention_days,
            orphan_grace_secs: config.orphan_grace.as_secs(),
            last_run_at: None,
            last_run_ms: 0,
            runs_total: 0,
            rooms_cut: 0,
            messages_removed: 0,
            attachment_rows_removed: 0,
            cursors_removed: 0,
            federated_index_rows_removed: 0,
            orphan_files_removed: 0,
            orphan_dirs_removed: 0,
            blobs_unlink_failed: 0,
            bytes_reclaimed: 0,
            last_error: None,
            stage: MaintenanceStage::Queued,
            classification: MaintenanceClassification::NotRun,
            accounting: MaintenanceAccounting::Unknown,
            in_progress_since: None,
            next_due_at: None,
        }
    }
}

/// Fixed report vocabulary; never classify using error or panic payload text.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum MaintenanceStage {
    #[default]
    Queued,
    Retention,
    OrphanGc,
    Publication,
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum MaintenanceClassification {
    #[default]
    NotRun,
    Ok,
    Failed,
    Cancelled,
    Panicked,
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum MaintenanceAccounting {
    #[default]
    Unknown,
    Complete,
    Incomplete,
}

/// The live report, shared between the sweep loop, the on-demand route, and
/// `/health`.
pub(super) struct MaintenanceState {
    report: Mutex<RoomMaintenanceReport>,
    sweep: Arc<tokio::sync::Mutex<()>>,
}

pub(super) type MaintenanceHandle = Arc<MaintenanceState>;

/// Build the shared report in its pre-first-sweep state.
pub(super) fn new_handle(config: &MaintenanceConfig) -> MaintenanceHandle {
    Arc::new(MaintenanceState {
        report: Mutex::new(RoomMaintenanceReport::new(config)),
        sweep: Arc::new(tokio::sync::Mutex::new(())),
    })
}

/// Read the current report, recovering a poisoned lock the way every other
/// registry in this daemon does. A poisoned maintenance mutex must never take
/// `/health` down with it — the card exists to make failure visible.
pub(super) fn report_snapshot(handle: &MaintenanceHandle) -> RoomMaintenanceReport {
    match handle.report.lock() {
        Ok(guard) => guard.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

/// The counts one sweep produced, before they are folded into the report.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct SweepOutcome {
    pub(super) rooms_cut: u64,
    pub(super) messages_removed: u64,
    pub(super) attachment_rows_removed: u64,
    pub(super) cursors_removed: u64,
    pub(super) federated_index_rows_removed: u64,
    pub(super) orphan_files_removed: u64,
    pub(super) orphan_dirs_removed: u64,
    pub(super) blobs_unlink_failed: u64,
    pub(super) bytes_reclaimed: u64,
    pub(super) error: Option<String>,
    stage: MaintenanceStage,
    failure_stage: Option<MaintenanceStage>,
    failure: Option<MaintenanceClassification>,
    accounting_unknown: bool,
}

// ---- The sweep --------------------------------------------------------------

/// Run both jobs once.
///
/// Synchronous and blocking: it takes the store lock repeatedly and walks a
/// directory tree, so callers put it on a blocking thread rather than a runtime
/// worker. Enumeration runs outside the store lock. Each destructive orphan
/// decision retains the store guard through its descriptor-relative unlink;
/// no store guard crosses an await.
///
/// `now` is a parameter and not `Utc::now()` so retention is testable against a
/// fixed clock: a test that had to sleep past a real window could only ever
/// exercise a window of zero days, which is the one value that means "off".
#[cfg(test)]
pub(super) fn run_sweep(
    rooms: &RoomStoreHandle,
    blob_root: &Path,
    config: &MaintenanceConfig,
    now: DateTime<Utc>,
) -> SweepOutcome {
    let mut outcome = SweepOutcome::default();
    run_sweep_into(rooms, blob_root, config, now, &mut outcome, &mut |_| {});
    outcome
}

/// The caller owns this accumulator outside its unwind boundary. Every known
/// committed count survives a later panic; incomplete counts are lower bounds.
fn run_sweep_into(
    rooms: &RoomStoreHandle,
    blob_root: &Path,
    config: &MaintenanceConfig,
    now: DateTime<Utc>,
    outcome: &mut SweepOutcome,
    stage_changed: &mut dyn FnMut(MaintenanceStage),
) {
    let orphan_started = std::time::SystemTime::now();
    outcome.stage = MaintenanceStage::Retention;
    stage_changed(outcome.stage);
    if let Err(error) = run_retention(rooms, blob_root, config, now, outcome) {
        outcome.error = Some(error);
    }
    if outcome.error.is_some() {
        outcome.failure_stage = Some(outcome.stage);
    }
    outcome.stage = MaintenanceStage::OrphanGc;
    stage_changed(outcome.stage);
    if let Err(error) = run_orphan_gc_at(
        rooms,
        blob_root,
        config,
        outcome,
        orphan_started,
        |_| {},
        |_| {},
    ) {
        outcome.error.get_or_insert(error);
    }
    if outcome.error.is_some() {
        outcome.failure_stage.get_or_insert(outcome.stage);
    }
}

/// Cut every room closed longer than the window.
fn run_retention(
    rooms: &RoomStoreHandle,
    blob_root: &Path,
    config: &MaintenanceConfig,
    now: DateTime<Utc>,
    outcome: &mut SweepOutcome,
) -> Result<(), String> {
    if config.retention_days == 0 {
        return Ok(());
    }
    let Some(window) = chrono::Duration::try_days(i64::from(config.retention_days)) else {
        return Err("retention window does not fit a duration".to_string());
    };
    let cutoff = now - window;
    let eligible = with_rooms_handle(rooms, |store| store.rooms_closed_before(cutoff))
        .map_err(|_| "retention could not list closed rooms".to_string())?;

    for key in eligible {
        // One transaction per room, not one for all of them. A single sweeping
        // transaction would hold the write lock across every room's deletes and
        // block live traffic for the whole sweep, and a failure on room 40 would
        // roll back the 39 cuts that were already correct.
        let _custody = crate::room_attachments::cleanup_guard();
        let dir_path = crate::room_attachments::room_dir(blob_root, &key);
        let root = crate::room_attachments::BlobDir::open_root(blob_root, false);
        let dir = root.as_ref().ok().and_then(|root| {
            root.open_child(dir_path.file_name().expect("hashed name"))
                .ok()
        });
        let cut = match with_rooms_handle(rooms, |store| store.cut_closed_room(&key)) {
            Ok(cut) => cut,
            Err(_) => {
                // Deliberately not the store's message: it names the room, and
                // this string is published on `/health`.
                outcome
                    .error
                    .get_or_insert_with(|| "retention failed to cut a closed room".to_string());
                continue;
            }
        };
        outcome.rooms_cut += 1;
        outcome.messages_removed += cut.messages_removed;
        outcome.attachment_rows_removed += cut.attachment_rows_removed;
        outcome.cursors_removed += cut.cursors_removed;
        outcome.federated_index_rows_removed += cut.federated_index_rows_removed;

        // Bytes AFTER the commit, exactly as `DELETE .../attachments/{id}` does
        // it. A blob whose unlink fails is still collectable — the row is gone,
        // so the orphan pass below sees an unreferenced file in a directory the
        // store still expects — but "a later sweep gets it" is not a reason to
        // discard the error, and the byte total is not allowed to claim it.
        //
        // `bytes_reclaimed` counts a blob only when the file is actually gone
        // (unlinked here; already absent earns zero). Adding the row's recorded
        // `byte_len` unconditionally would let the report announce reclaimed
        // disk on a tree that never gave any back, which is the exact failure
        // this card exists to make visible rather than to paper over.
        for (id, _recorded_len) in cut.attachment_blobs {
            if crate::room_attachments::blob_path(blob_root, &key, &id).is_none() {
                outcome.blobs_unlink_failed += 1;
                outcome.error.get_or_insert_with(|| {
                    "retention refused a malformed stored attachment id".into()
                });
                continue;
            }
            let removed = match &dir {
                Some(dir) => dir
                    .entry_metadata(id.as_ref())
                    .and_then(|metadata| dir.remove_measured(id.as_ref(), &metadata)),
                None => match &root {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Ok(root) => {
                        match root.entry_metadata(dir_path.file_name().expect("hashed name")) {
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                            _ => Err(std::io::Error::other("unsafe room directory")),
                        }
                    }
                    _ => Err(std::io::Error::other("unsafe attachment root")),
                },
            };
            match removed {
                Ok((bytes, synced)) => {
                    outcome.bytes_reclaimed += bytes;
                    if !synced {
                        outcome.error.get_or_insert_with(|| {
                            "retention could not sync attachment cleanup".into()
                        });
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => {
                    outcome.blobs_unlink_failed += 1;
                    outcome.error.get_or_insert_with(|| {
                        "retention could not unlink a cut room's attachment bytes".into()
                    });
                }
            }
        }
        if let (Ok(root), Some(dir)) = (&root, &dir) {
            let _ = root.remove_empty_child(dir_path.file_name().expect("hashed name"), dir);
        }
    }
    Ok(())
}

/// Classifications are refreshed under publication exclusion and the store guard
/// immediately before removal. The directory walk never authorizes deletion.
fn run_orphan_gc_at(
    rooms: &RoomStoreHandle,
    blob_root: &Path,
    config: &MaintenanceConfig,
    outcome: &mut SweepOutcome,
    started: std::time::SystemTime,
    mut before_candidate: impl FnMut(&Path),
    mut after_capture: impl FnMut(&Path),
) -> Result<(), String> {
    let Some(cutoff) = started.checked_sub(config.orphan_grace) else {
        return Ok(());
    };
    let root = match crate::room_attachments::BlobDir::open_root(blob_root, false) {
        Ok(root) => root,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err("orphan GC could not read the attachment root".into()),
    };
    let entries = root
        .names()
        .map_err(|_| "orphan GC could not enumerate attachment root".to_string())?;
    for entry in entries {
        let Some(name) = gc_inspected(
            entry,
            outcome,
            "orphan GC could not enumerate an attachment root entry",
        ) else {
            continue;
        };
        let path = blob_root.join(&name);
        before_candidate(&path);
        // Busy publication is a reason to defer cleanup, never to infer orphanhood.
        let Some(_custody) = crate::room_attachments::try_cleanup_guard() else {
            continue;
        };
        let Some(metadata) = gc_inspected(
            root.entry_metadata(&name),
            outcome,
            "orphan GC refused an unsafe attachment root entry",
        ) else {
            continue;
        };
        if !metadata.is_dir() {
            continue;
        }
        let Some(dir) = gc_inspected(
            root.open_child(&name),
            outcome,
            "orphan GC could not open a room directory",
        ) else {
            continue;
        };
        if !dir.matches_metadata(&metadata).unwrap_or(false) {
            outcome
                .error
                .get_or_insert_with(|| "orphan GC refused a replaced room directory".into());
            continue;
        }
        after_capture(&path);
        let Some(entries) = gc_inspected(
            dir.names(),
            outcome,
            "orphan GC could not read a room directory",
        ) else {
            continue;
        };
        for entry in entries {
            let Some(file) = gc_inspected(
                entry,
                outcome,
                "orphan GC could not enumerate a room directory entry",
            ) else {
                continue;
            };
            let Some(metadata) = gc_inspected(
                dir.entry_metadata(&file),
                outcome,
                "orphan GC refused an unsafe room entry",
            ) else {
                continue;
            };
            if !metadata.is_file() {
                outcome
                    .error
                    .get_or_insert_with(|| "orphan GC refused an unsafe room entry".into());
                continue;
            }
            if !older_than_cutoff(&metadata, cutoff) {
                continue;
            }
            let removed = with_rooms_handle(rooms, |store| {
                // Re-derive even unknown ownership for each destructive decision:
                // a room may have been created since directory enumeration.
                let key = store.room_keys_including_closed()?.into_iter().find(|key| {
                    crate::room_attachments::room_dir(blob_root, key).file_name()
                        == Some(name.as_os_str())
                });
                if let Some(key) = key {
                    if let Some(id) = file.to_str() {
                        if store.attachment(&key, id)?.is_some() {
                            return Ok(None);
                        }
                    }
                }
                Ok::<_, ocean_store::RoomStoreError>(Some(dir.remove_measured(&file, &metadata)))
            });
            match removed {
                Ok(Some(Ok((bytes, synced)))) => {
                    outcome.orphan_files_removed += 1;
                    outcome.bytes_reclaimed += bytes;
                    if !synced {
                        outcome
                            .error
                            .get_or_insert_with(|| "orphan GC could not sync blob cleanup".into());
                    }
                }
                Ok(Some(Err(error))) if error.kind() == std::io::ErrorKind::NotFound => {}
                Ok(Some(Err(_))) => {
                    outcome.blobs_unlink_failed += 1;
                    outcome
                        .error
                        .get_or_insert_with(|| "orphan GC could not unlink a blob".into());
                }
                Ok(None) => {}
                Err(_) => {
                    outcome.error.get_or_insert_with(|| {
                        "orphan GC could not read live attachment references".into()
                    });
                }
            }
        }
        // Young, referenced or unsafe entries can legitimately keep a directory
        // nonempty. Do not report a removal failure for intentionally kept bytes.
        match dir
            .names()
            .and_then(|mut entries| entries.next().transpose())
        {
            Ok(Some(_)) => continue,
            Err(_) => {
                outcome.error.get_or_insert_with(|| {
                    "orphan GC could not confirm an empty room directory".into()
                });
                continue;
            }
            Ok(None) => {}
        }
        // Removing only an empty captured directory cannot recursively consume a
        // newly published file. Unknown ownership is rechecked while store-locked.
        let result = with_rooms_handle(rooms, |store| {
            let known = store.room_keys_including_closed()?.iter().any(|key| {
                crate::room_attachments::room_dir(blob_root, key).file_name()
                    == Some(name.as_os_str())
            });
            if known || !older_than_cutoff(&metadata, cutoff) {
                return Ok(None);
            }
            Ok::<_, ocean_store::RoomStoreError>(Some(root.remove_empty_child(&name, &dir)))
        });
        match result {
            Ok(Some(Ok(synced))) => {
                outcome.orphan_dirs_removed += 1;
                if !synced {
                    outcome
                        .error
                        .get_or_insert_with(|| "orphan GC could not sync directory cleanup".into());
                }
            }
            Ok(Some(Err(error))) if error.kind() == std::io::ErrorKind::NotFound => {}
            Ok(Some(Err(_))) => {
                outcome.error.get_or_insert_with(|| {
                    "orphan GC could not remove an unrecognized room directory".into()
                });
            }
            Ok(None) => {}
            Err(_) => {
                outcome
                    .error
                    .get_or_insert_with(|| "orphan GC could not read live room ownership".into());
            }
        }
    }
    Ok(())
}

fn older_than_cutoff(metadata: &std::fs::Metadata, cutoff: std::time::SystemTime) -> bool {
    metadata
        .modified()
        .map(|modified| modified <= cutoff)
        .unwrap_or(false)
}

/// Preserve the first bounded failure while allowing unrelated entries to be
/// inspected. Never expose filesystem paths or raw operating-system errors.
fn gc_inspected<T>(
    result: std::io::Result<T>,
    outcome: &mut SweepOutcome,
    error: &'static str,
) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(_) => {
            outcome.error.get_or_insert_with(|| error.to_string());
            None
        }
    }
}

// ---- Loop + on-demand route -------------------------------------------------

/// Publish terminal facts only. No logging or external callbacks may run here.
fn record_sweep(
    handle: &MaintenanceHandle,
    outcome: &SweepOutcome,
    finished_at: DateTime<Utc>,
    elapsed: Duration,
) -> RoomMaintenanceReport {
    let mut guard = match handle.report.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    guard.last_run_at = Some(finished_at.to_rfc3339());
    guard.last_run_ms = elapsed.as_millis().min(u128::from(u64::MAX)) as u64;
    guard.runs_total = guard.runs_total.saturating_add(1);
    guard.rooms_cut = outcome.rooms_cut;
    guard.messages_removed = outcome.messages_removed;
    guard.attachment_rows_removed = outcome.attachment_rows_removed;
    guard.cursors_removed = outcome.cursors_removed;
    guard.federated_index_rows_removed = outcome.federated_index_rows_removed;
    guard.orphan_files_removed = outcome.orphan_files_removed;
    guard.orphan_dirs_removed = outcome.orphan_dirs_removed;
    guard.blobs_unlink_failed = outcome.blobs_unlink_failed;
    guard.bytes_reclaimed = outcome.bytes_reclaimed;
    guard.last_error = outcome.error.clone();
    guard.stage = outcome.failure_stage.unwrap_or(outcome.stage);
    guard.classification = outcome.failure.unwrap_or(if outcome.error.is_some() {
        MaintenanceClassification::Failed
    } else {
        MaintenanceClassification::Ok
    });
    guard.accounting = if outcome.accounting_unknown {
        MaintenanceAccounting::Unknown
    } else if outcome.error.is_some() {
        MaintenanceAccounting::Incomplete
    } else {
        MaintenanceAccounting::Complete
    };
    guard.in_progress_since = None;
    guard.clone()
}

fn log_sweep(outcome: &SweepOutcome, retention_days: u32, elapsed: Duration) {
    // One line per run, at info, with every number the card carries. The card
    // says what is true NOW; this line is the history, and it is what an
    // operator has when the question is "when did the disk actually come back".
    tracing::info!(
        rooms_cut = outcome.rooms_cut,
        messages_removed = outcome.messages_removed,
        attachment_rows_removed = outcome.attachment_rows_removed,
        cursors_removed = outcome.cursors_removed,
        federated_index_rows_removed = outcome.federated_index_rows_removed,
        orphan_files_removed = outcome.orphan_files_removed,
        orphan_dirs_removed = outcome.orphan_dirs_removed,
        blobs_unlink_failed = outcome.blobs_unlink_failed,
        bytes_reclaimed = outcome.bytes_reclaimed,
        retention_days,
        elapsed_ms = elapsed.as_millis().min(u128::from(u64::MAX)) as u64,
        error = outcome.error.as_deref().unwrap_or("none"),
        "room maintenance sweep finished"
    );
}

/// Run one sweep off the runtime workers and record it.
///
/// `spawn_blocking` because the sweep takes a `std::sync::Mutex` and walks a
/// directory tree; doing that on a runtime worker is how a large blob tree
/// becomes a stalled daemon.
struct SweepRun {
    // Shared with the join waiter: even a worker panic cannot release custody
    // before its fallback publishes. Caller cancellation still leaves the
    // blocking worker owning both the permit and its completion.
    _permit: tokio::sync::OwnedMutexGuard<()>,
    started: std::time::Instant,
    progress: Mutex<SweepProgress>,
}

#[derive(Default)]
struct SweepProgress {
    outcome: SweepOutcome,
    recorded: bool,
}

async fn start_sweep(handle: &MaintenanceHandle) -> Arc<SweepRun> {
    let permit = handle.sweep.clone().lock_owned().await;
    let run = Arc::new(SweepRun {
        _permit: permit,
        started: std::time::Instant::now(),
        progress: Mutex::new(SweepProgress::default()),
    });
    let mut report = handle
        .report
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    report.in_progress_since = Some(Utc::now().to_rfc3339());
    report.stage = MaintenanceStage::Queued;
    run
}

fn finish_sweep(
    handle: &MaintenanceHandle,
    run: &SweepRun,
    progress: &mut SweepProgress,
) -> RoomMaintenanceReport {
    if progress.recorded {
        return report_snapshot(handle);
    }
    progress.recorded = true;
    record_sweep(handle, &progress.outcome, Utc::now(), run.started.elapsed())
}

fn sweep_worker(
    handle: &MaintenanceHandle,
    run: &SweepRun,
    work: impl FnOnce(&mut SweepOutcome, &mut dyn FnMut(MaintenanceStage)),
    publish_log: impl FnOnce(&SweepOutcome, Duration),
) -> RoomMaintenanceReport {
    let mut progress = run
        .progress
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let mut stage_changed = |stage| {
        handle
            .report
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .stage = stage;
    };
    // Sweep, stage publication and logging share the boundary. The accumulator
    // lives outside it, and terminal fallback never calls the failing logger.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        work(&mut progress.outcome, &mut stage_changed);
        progress.outcome.stage = MaintenanceStage::Publication;
        stage_changed(MaintenanceStage::Publication);
        publish_log(&progress.outcome, run.started.elapsed());
    }));
    if result.is_err() {
        let outcome = &mut progress.outcome;
        outcome.error = Some("room maintenance worker panicked".into());
        outcome.failure = Some(MaintenanceClassification::Panicked);
        outcome.failure_stage = Some(outcome.stage);
    }
    finish_sweep(handle, run, &mut progress)
}

async fn await_sweep(
    handle: &MaintenanceHandle,
    run: &SweepRun,
    worker: tokio::task::JoinHandle<RoomMaintenanceReport>,
) -> RoomMaintenanceReport {
    match worker.await {
        Ok(report) => report,
        Err(error) => {
            let mut progress = run
                .progress
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            // A queued cancellation or exceptional worker failure cannot assert
            // complete accounting. Preserve whatever progress is known, publish
            // exactly once, and never run tracing on this fallback path.
            progress.outcome.error = Some(if error.is_cancelled() {
                "room maintenance worker cancelled".into()
            } else {
                "room maintenance worker panicked".into()
            });
            progress.outcome.failure = Some(if error.is_cancelled() {
                MaintenanceClassification::Cancelled
            } else {
                MaintenanceClassification::Panicked
            });
            progress.outcome.failure_stage = Some(progress.outcome.stage);
            progress.outcome.accounting_unknown = true;
            finish_sweep(handle, run, &mut progress)
        }
    }
}

async fn sweep_once_with(
    handle: MaintenanceHandle,
    work: impl FnOnce(&mut SweepOutcome, &mut dyn FnMut(MaintenanceStage)) + Send + 'static,
    publish_log: impl FnOnce(&SweepOutcome, Duration) + Send + 'static,
) -> RoomMaintenanceReport {
    let run = start_sweep(&handle).await;
    let worker_run = run.clone();
    let worker_handle = handle.clone();
    let worker = tokio::task::spawn_blocking(move || {
        sweep_worker(&worker_handle, &worker_run, work, publish_log)
    });
    await_sweep(&handle, &run, worker).await
}

async fn sweep_once(
    rooms: RoomStoreHandle,
    blob_root: Arc<PathBuf>,
    config: MaintenanceConfig,
    handle: MaintenanceHandle,
) -> RoomMaintenanceReport {
    sweep_once_with(
        handle,
        move |outcome, stage_changed| {
            run_sweep_into(
                &rooms,
                &blob_root,
                &config,
                Utc::now(),
                outcome,
                stage_changed,
            );
        },
        move |outcome, elapsed| log_sweep(outcome, config.retention_days, elapsed),
    )
    .await
}

/// A blocked worker remains visibly in progress and retains cleanup custody.
/// Do not add a timeout which admits another sweep while that worker runs.
///
/// Each iteration runs as its own task, so a panic anywhere in it (including
/// the async join fallback and its publication) ends that iteration, never the
/// scheduler. Cadence is fixed-delay: the next sweep is due one interval after
/// the previous one settled, which is what `next_due_at` reports.
async fn maintenance_loop<F, R>(handle: MaintenanceHandle, interval: Duration, mut sweep: F)
where
    F: FnMut() -> R,
    R: std::future::Future<Output = RoomMaintenanceReport> + Send + 'static,
{
    let mut ticker = tokio::time::interval(interval);
    ticker.tick().await;
    settle_iteration(&handle, interval, false);
    loop {
        ticker.tick().await;
        let panicked = tokio::spawn(sweep()).await.is_err();
        ticker.reset();
        settle_iteration(&handle, interval, panicked);
    }
}

/// Scheduler bookkeeping after one iteration. Fenced by its own unwind
/// boundary and free of logging, so it cannot become the path that ends the
/// loop.
fn settle_iteration(handle: &MaintenanceHandle, interval: Duration, panicked: bool) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if panicked {
            settle_abandoned_sweep(handle);
        }
        let next_due = chrono::Duration::from_std(interval)
            .ok()
            .and_then(|interval| Utc::now().checked_add_signed(interval))
            .map(|due| due.to_rfc3339());
        handle
            .report
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .next_due_at = next_due;
    }));
}

/// Close out an iteration that panicked before publishing its outcome.
///
/// Records exactly once: a worker still holding custody publishes its own
/// outcome, and a sweep that already published has cleared
/// `in_progress_since`. The iteration's progress died with it, so counts are
/// the only known lower bound (zero) and accounting is `unknown`.
fn settle_abandoned_sweep(handle: &MaintenanceHandle) {
    let Ok(_custody) = handle.sweep.try_lock() else {
        return;
    };
    let mut report = handle
        .report
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if report.in_progress_since.take().is_none() {
        return;
    }
    let stage = report.stage;
    let outcome = SweepOutcome {
        error: Some("room maintenance loop iteration panicked".into()),
        stage,
        failure_stage: Some(stage),
        failure: Some(MaintenanceClassification::Panicked),
        accounting_unknown: true,
        ..SweepOutcome::default()
    };
    drop(report);
    record_sweep(handle, &outcome, Utc::now(), Duration::ZERO);
}

pub(super) fn spawn_maintenance_loop(state: &AppState) {
    let rooms = state.rooms.clone();
    let blob_root = state.room_attachments_root.clone();
    let config = state.room_maintenance_config;
    let handle = state.room_maintenance.clone();
    tokio::spawn(async move {
        maintenance_loop(handle.clone(), config.interval, || {
            sweep_once(rooms.clone(), blob_root.clone(), config, handle.clone())
        })
        .await;
    });
}

/// `POST /v1/rooms/maintenance/run` — sweep now, operator only.
///
/// Operator-authenticated for the same reason the room-agent mutations are: this
/// route deletes durable content on demand, and the local trust boundary this
/// daemon actually has is the `X-Ocean-Operator` key — not membership in any one
/// room, since the sweep is store-wide and belongs to no room.
///
/// It answers with the report the sweep just wrote, so an operator running it by
/// hand does not then have to go to `/health` to find out what happened.
pub(super) async fn room_maintenance_run(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<serde_json::Value>) {
    // No member lane here, unlike `POST .../close`: there is no room to be a
    // member of. An absent header is therefore a refusal and not a fallback,
    // which is exactly the mapping `room_agent_authority` uses.
    let principal = match state.room_operator.authorize(&headers) {
        Ok(principal) => principal,
        Err(error) => return crate::room_agent_authority::ApiError::from(error).response(),
    };
    tracing::info!(
        operator = principal.id(),
        "room maintenance sweep requested on demand"
    );
    let report = sweep_once(
        state.rooms.clone(),
        state.room_attachments_root.clone(),
        state.room_maintenance_config,
        state.room_maintenance.clone(),
    )
    .await;
    (
        StatusCode::OK,
        Json(json!({ "ok": true, "room_maintenance": report })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocean_core::{RoomKey, RoomMessageKind, RoomParticipant, RoomParticipantKind};
    use ocean_store::{RoomCloser, RoomStore, SqliteRoomStore};

    fn test_log_sweep(outcome: &SweepOutcome, elapsed: Duration) {
        log_sweep(outcome, 0, elapsed);
    }

    #[tokio::test]
    async fn partial_committed_retention_survives_panic_and_clean_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let (rooms, root) = fixture(dir.path());
        let now = Utc::now();
        let old = now - chrono::Duration::days(40);
        let key = RoomKey::new("private-fixture-room");
        seed_room(&rooms, &key, old);
        with_rooms_handle(&rooms, |store| {
            store.close_with_marker(&key, RoomCloser::Member("alice"), old)
        })
        .unwrap();
        let config = MaintenanceConfig {
            retention_days: 30,
            ..MaintenanceConfig::default()
        };
        let handle = new_handle(&config);
        let worker_rooms = rooms.clone();
        let worker_root = root.clone();
        let report = sweep_once_with(
            handle.clone(),
            move |outcome, changed| {
                run_sweep_into(
                    &worker_rooms,
                    &worker_root,
                    &config,
                    now,
                    outcome,
                    &mut |stage| {
                        changed(stage);
                        if stage == MaintenanceStage::OrphanGc {
                            panic!("fixture private payload must not be reported");
                        }
                    },
                );
            },
            test_log_sweep,
        )
        .await;
        assert_eq!(report.runs_total, 1);
        assert_eq!(report.rooms_cut, 1);
        assert_eq!(report.messages_removed, 3);
        assert_eq!(report.stage, MaintenanceStage::OrphanGc);
        assert_eq!(report.classification, MaintenanceClassification::Panicked);
        assert_eq!(report.accounting, MaintenanceAccounting::Incomplete);
        assert!(report.in_progress_since.is_none());
        let wire = serde_json::to_string(&report).unwrap();
        assert!(!wire.contains("private"));
        with_rooms_handle(&rooms, |store| {
            assert!(store
                .transcript_page_including_closed(&key, None, None)?
                .messages
                .is_empty());
            Ok::<_, ocean_store::RoomStoreError>(())
        })
        .unwrap();
        let clean = sweep_once(rooms, Arc::new(root), config, handle).await;
        assert_eq!(clean.runs_total, 2);
        assert_eq!(clean.classification, MaintenanceClassification::Ok);
        assert_eq!(clean.accounting, MaintenanceAccounting::Complete);
        assert_eq!(clean.last_error, None);
    }

    /// Wait until the scheduler has republished `next_due_at` after an
    /// iteration. Only yields: callers first await any blocking worker's
    /// report, so what remains is runtime-local scheduler work.
    async fn settled(
        handle: &MaintenanceHandle,
        previous: &Option<String>,
    ) -> RoomMaintenanceReport {
        for _ in 0..10_000 {
            let report = report_snapshot(handle);
            if report.next_due_at.is_some() && &report.next_due_at != previous {
                return report;
            }
            tokio::task::yield_now().await;
        }
        panic!("scheduler did not settle the iteration");
    }

    #[tokio::test(start_paused = true)]
    async fn publication_panic_does_not_double_count_or_stop_next_scheduler_tick() {
        let config = MaintenanceConfig::default();
        let handle = new_handle(&config);
        let task_handle = handle.clone();
        let loop_handle = handle.clone();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut attempt = 0;
        let task = tokio::spawn(async move {
            maintenance_loop(loop_handle, Duration::from_secs(60), || {
                attempt += 1;
                let first = attempt == 1;
                let handle = task_handle.clone();
                let tx = tx.clone();
                async move {
                    let report = sweep_once_with(
                        handle,
                        |outcome, _| {
                            outcome.rooms_cut = 2;
                            outcome.messages_removed = 17;
                            outcome.bytes_reclaimed = 4096;
                        },
                        move |_, _| {
                            if first {
                                panic!("fixed publication fixture");
                            }
                        },
                    )
                    .await;
                    tx.send(report.clone()).unwrap();
                    report
                }
            })
            .await;
        });
        let idle = settled(&handle, &None).await;
        tokio::time::advance(Duration::from_secs(60)).await;
        let failed = rx.recv().await.unwrap();
        assert_eq!(
            (
                failed.runs_total,
                failed.rooms_cut,
                failed.messages_removed,
                failed.bytes_reclaimed
            ),
            (1, 2, 17, 4096)
        );
        assert_eq!(failed.stage, MaintenanceStage::Publication);
        assert_eq!(failed.classification, MaintenanceClassification::Panicked);
        assert_eq!(failed.accounting, MaintenanceAccounting::Incomplete);
        let after_failed = settled(&handle, &idle.next_due_at).await;
        assert_eq!(after_failed.runs_total, 1);
        tokio::time::advance(Duration::from_secs(60)).await;
        let clean = rx.recv().await.unwrap();
        assert_eq!(clean.runs_total, 2);
        assert_eq!(clean.last_error, None);
        assert_eq!(clean.accounting, MaintenanceAccounting::Complete);
        assert_eq!(clean.classification, MaintenanceClassification::Ok);
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
    }

    type ReportTx = tokio::sync::mpsc::UnboundedSender<RoomMaintenanceReport>;
    type ReportRx = tokio::sync::mpsc::UnboundedReceiver<RoomMaintenanceReport>;

    /// Drive `maintenance_loop` where the first iteration's future is `first`
    /// and every later one is a clean sweep cutting one room. Clean iterations
    /// send their report once their blocking worker has finished.
    fn spawn_loop_with_first_iteration<F, Fut>(
        handle: &MaintenanceHandle,
        first: F,
    ) -> (tokio::task::JoinHandle<()>, ReportRx)
    where
        F: FnOnce(MaintenanceHandle, ReportTx) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = RoomMaintenanceReport> + Send + 'static,
    {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let loop_handle = handle.clone();
        let task_handle = handle.clone();
        let mut first = Some(first);
        let task = tokio::spawn(async move {
            maintenance_loop(loop_handle, Duration::from_secs(60), move || {
                let handle = task_handle.clone();
                let tx = tx.clone();
                let first = first.take().map(|first| first(handle.clone(), tx.clone()));
                async move {
                    match first {
                        Some(first) => first.await,
                        None => {
                            let report = sweep_once_with(
                                handle,
                                |outcome, _| outcome.rooms_cut = 1,
                                test_log_sweep,
                            )
                            .await;
                            tx.send(report.clone()).unwrap();
                            report
                        }
                    }
                }
            })
            .await;
        });
        (task, rx)
    }

    #[tokio::test(start_paused = true)]
    async fn panic_in_async_publication_path_is_recorded_once_and_next_tick_runs() {
        let handle = new_handle(&MaintenanceConfig::default());
        // Panic on the loop side after custody and `in_progress_since` are
        // taken but before any outcome is published: the path the deployed
        // JoinError fallback took when its own publication re-panicked.
        let (task, mut rx) = spawn_loop_with_first_iteration(&handle, |handle, _| async move {
            let _run = start_sweep(&handle).await;
            panic!("private async publication payload");
        });
        let idle = settled(&handle, &None).await;
        assert_eq!(idle.runs_total, 0);
        tokio::time::advance(Duration::from_secs(60)).await;
        let failed = settled(&handle, &idle.next_due_at).await;
        assert_eq!(failed.runs_total, 1, "abandoned iteration counted once");
        assert_eq!(failed.classification, MaintenanceClassification::Panicked);
        assert_eq!(failed.accounting, MaintenanceAccounting::Unknown);
        assert_eq!(
            failed.last_error.as_deref(),
            Some("room maintenance loop iteration panicked")
        );
        assert_eq!(failed.rooms_cut, 0);
        assert!(failed.in_progress_since.is_none());
        assert!(failed.last_run_at.is_some());
        assert!(!serde_json::to_string(&failed).unwrap().contains("private"));
        assert!(handle.sweep.try_lock().is_ok(), "custody released");

        tokio::time::advance(Duration::from_secs(60)).await;
        rx.recv().await.unwrap();
        let clean = settled(&handle, &failed.next_due_at).await;
        assert_eq!(clean.runs_total, 2, "scheduler ran its next tick");
        assert_eq!(clean.classification, MaintenanceClassification::Ok);
        assert_eq!(clean.accounting, MaintenanceAccounting::Complete);
        assert_eq!(clean.rooms_cut, 1);
        assert_eq!(clean.last_error, None);
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
    }

    #[tokio::test(start_paused = true)]
    async fn panic_after_publication_does_not_double_count_and_next_tick_runs() {
        let handle = new_handle(&MaintenanceConfig::default());
        let (task, mut rx) = spawn_loop_with_first_iteration(&handle, |handle, tx| async move {
            let report = sweep_once_with(
                handle,
                |outcome, _| outcome.messages_removed = 17,
                test_log_sweep,
            )
            .await;
            tx.send(report).unwrap();
            panic!("fixed post-publication fixture");
        });
        let idle = settled(&handle, &None).await;
        tokio::time::advance(Duration::from_secs(60)).await;
        rx.recv().await.unwrap();
        let first = settled(&handle, &idle.next_due_at).await;
        assert_eq!(first.runs_total, 1);
        assert_eq!(first.messages_removed, 17);
        assert_eq!(first.classification, MaintenanceClassification::Ok);
        assert_eq!(first.accounting, MaintenanceAccounting::Complete);
        tokio::time::advance(Duration::from_secs(60)).await;
        rx.recv().await.unwrap();
        let next = settled(&handle, &first.next_due_at).await;
        assert_eq!(next.runs_total, 2);
        assert_eq!(next.rooms_cut, 1);
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
    }

    #[tokio::test]
    async fn exceptional_join_panic_keeps_progress_and_finishes_only_once() {
        let handle = new_handle(&MaintenanceConfig::default());
        let run = start_sweep(&handle).await;
        let worker_run = run.clone();
        let worker = tokio::task::spawn_blocking(move || -> RoomMaintenanceReport {
            let mut progress = worker_run.progress.lock().unwrap();
            progress.outcome.rooms_cut = 1;
            progress.outcome.stage = MaintenanceStage::Retention;
            panic!("private join failure payload");
        });
        let report = await_sweep(&handle, &run, worker).await;
        assert_eq!((report.runs_total, report.rooms_cut), (1, 1));
        assert_eq!(report.classification, MaintenanceClassification::Panicked);
        assert_eq!(report.stage, MaintenanceStage::Retention);
        assert_eq!(report.accounting, MaintenanceAccounting::Unknown);
        assert!(!serde_json::to_string(&report).unwrap().contains("private"));
        let mut progress = run
            .progress
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        assert_eq!(finish_sweep(&handle, &run, &mut progress), report);
        assert!(
            handle.sweep.try_lock().is_err(),
            "fallback retains custody through completion"
        );
    }

    #[test]
    fn cancelled_queued_worker_reports_fixed_unknown_and_allows_next_sweep() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .max_blocking_threads(1)
            .build()
            .unwrap();
        runtime.block_on(async {
            let (started_tx, started_rx) = std::sync::mpsc::channel();
            let (release_tx, release_rx) = std::sync::mpsc::channel();
            let blocker = tokio::task::spawn_blocking(move || {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            });
            started_rx.recv().unwrap();
            let handle = new_handle(&MaintenanceConfig::default());
            let run = start_sweep(&handle).await;
            assert!(report_snapshot(&handle).in_progress_since.is_some());
            let worker = tokio::task::spawn_blocking(|| -> RoomMaintenanceReport {
                panic!("aborted queued fixture must not execute");
            });
            worker.abort();
            // Abort while queued, then let the blocking pool dequeue it so the
            // JoinHandle can settle without depending on cancellation timing.
            release_tx.send(()).unwrap();
            let cancelled = await_sweep(&handle, &run, worker).await;
            assert_eq!(cancelled.runs_total, 1);
            assert_eq!(
                cancelled.classification,
                MaintenanceClassification::Cancelled
            );
            assert_eq!(cancelled.stage, MaintenanceStage::Queued);
            assert_eq!(cancelled.accounting, MaintenanceAccounting::Unknown);
            assert_eq!(cancelled.rooms_cut, 0);
            assert!(cancelled.in_progress_since.is_none());
            drop(run);
            blocker.await.unwrap();
            let clean = sweep_once_with(handle, |_, _| {}, test_log_sweep).await;
            assert_eq!(clean.runs_total, 2);
            assert_eq!(clean.classification, MaintenanceClassification::Ok);
        });
    }

    #[test]
    fn enumeration_failure_is_bounded_and_does_not_skip_later_entries() {
        let mut outcome = SweepOutcome::default();
        let entries = [Ok(1), Err(std::io::Error::other("private/path")), Ok(2)];
        let mut visited = Vec::new();
        for entry in entries {
            if let Some(value) = gc_inspected(entry, &mut outcome, "bounded enumeration failure") {
                visited.push(value);
            }
        }
        assert_eq!(visited, [1, 2]);
        assert_eq!(
            outcome.error.as_deref(),
            Some("bounded enumeration failure")
        );
        assert!(gc_inspected::<()>(
            Err(std::io::Error::other("secret")),
            &mut outcome,
            "second failure"
        )
        .is_none());
        assert_eq!(
            outcome.error.as_deref(),
            Some("bounded enumeration failure")
        );
    }

    #[tokio::test]
    async fn concurrent_sweeps_return_their_own_serial_report() {
        let dir = tempfile::tempdir().unwrap();
        let (rooms, root) = fixture(dir.path());
        let root = Arc::new(root);
        let config = MaintenanceConfig::default();
        let handle = new_handle(&config);
        let (first, second) = tokio::join!(
            sweep_once(rooms.clone(), root.clone(), config, handle.clone()),
            sweep_once(rooms, root, config, handle.clone())
        );
        let mut runs = [first.runs_total, second.runs_total];
        runs.sort();
        assert_eq!(runs, [1, 2]);
        assert_eq!(report_snapshot(&handle).runs_total, 2);
    }

    #[tokio::test]
    async fn cancelled_sweep_request_retains_worker_ownership_and_records_completion() {
        let dir = tempfile::tempdir().unwrap();
        let (rooms, root) = fixture(dir.path());
        std::fs::create_dir_all(root.join("0".repeat(64))).unwrap();
        let root = Arc::new(root);
        let config = MaintenanceConfig::default();
        let handle = new_handle(&config);
        let (locked_tx, locked_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let held_rooms = rooms.clone();
        let holder = std::thread::spawn(move || {
            let _guard = held_rooms.lock().unwrap();
            locked_tx.send(()).unwrap();
            release_rx.blocking_recv().unwrap();
        });
        locked_rx.await.unwrap();
        let request = tokio::spawn(sweep_once(
            rooms.clone(),
            root.clone(),
            config,
            handle.clone(),
        ));
        tokio::time::timeout(Duration::from_secs(2), async {
            while handle.sweep.try_lock().is_ok() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        request.abort();
        assert!(request.await.unwrap_err().is_cancelled());
        assert!(
            handle.sweep.try_lock().is_err(),
            "the detached worker still owns the sweep"
        );
        assert_eq!(report_snapshot(&handle).runs_total, 0);
        assert!(report_snapshot(&handle).in_progress_since.is_some());
        release_tx.send(()).unwrap();
        holder.join().unwrap();
        let next = sweep_once(rooms, root, config, handle.clone()).await;
        assert_eq!(
            next.runs_total, 2,
            "the cancelled request's sweep was recorded before the next run"
        );
        assert_eq!(next.last_error, None);
    }

    /// A store handle plus the blob root that indexes into it, laid out the way
    /// the daemon lays them out: `<dir>/rooms.db` beside `<dir>/room-attachments`.
    fn fixture(dir: &Path) -> (RoomStoreHandle, PathBuf) {
        let store = SqliteRoomStore::open(dir.join("rooms.db")).expect("open store");
        (Arc::new(Mutex::new(store)), dir.join("room-attachments"))
    }

    /// Create an open room with one Human on the roster and one ordinary
    /// message in it.
    fn seed_room(rooms: &RoomStoreHandle, key: &RoomKey, at: DateTime<Utc>) {
        with_rooms_handle(rooms, |store| {
            store.create(key.clone(), "Fixture", None, at)?;
            store.add_participant(
                key,
                RoomParticipant {
                    id: "alice".into(),
                    kind: RoomParticipantKind::Human,
                    display_name: "Alice".into(),
                },
                at,
            )?;
            store.append_message(
                key,
                "alice",
                RoomParticipantKind::Human,
                RoomMessageKind::Message,
                "a durable line",
                at,
            )?;
            Ok::<_, ocean_store::RoomStoreError>(())
        })
        .expect("seed room");
    }

    /// Index an attachment AND write its bytes, exactly as an upload does.
    fn seed_attachment(
        rooms: &RoomStoreHandle,
        blob_root: &Path,
        key: &RoomKey,
        id: &str,
        bytes: &[u8],
        at: DateTime<Utc>,
    ) {
        crate::room_attachments::write_blob_for_test(blob_root, key, id, bytes);
        with_rooms_handle(rooms, |store| {
            store.add_attachment(
                key,
                id,
                "notes.txt",
                "text/plain",
                bytes.len() as u64,
                "0".repeat(64).as_str(),
                "alice",
                at,
            )
        })
        .expect("index attachment");
    }

    /// Retention cuts a room closed longer than the window, and NOTHING else.
    ///
    /// A fixed clock rather than a real one, because the only window a test
    /// could reach by waiting is zero days — and zero is the value that means
    /// "retention is off". Three rooms, one for each way the window is decided:
    /// closed before the cutoff, closed after it, and never closed at all. The
    /// open room is the assertion that matters most: retention is measured from
    /// the CLOSE, so a room that has been running for a year is not eligible,
    /// and a sweep that measured from `created_at` or `updated_at` instead would
    /// pass the first two assertions and delete a live room's history.
    #[test]
    fn retention_cuts_only_a_room_closed_longer_than_the_window() {
        let tmp = tempfile::tempdir().unwrap();
        let (rooms, blob_root) = fixture(tmp.path());
        let now = DateTime::parse_from_rfc3339("2026-09-02T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let long_ago = now - chrono::Duration::days(40);
        let recently = now - chrono::Duration::days(2);

        let cut = RoomKey::new("closed-long-ago");
        let keep = RoomKey::new("closed-recently");
        let open = RoomKey::new("never-closed");
        for key in [&cut, &keep, &open] {
            seed_room(&rooms, key, long_ago);
        }
        seed_attachment(
            &rooms,
            &blob_root,
            &cut,
            &"a".repeat(32),
            b"cut me",
            long_ago,
        );
        seed_attachment(
            &rooms,
            &blob_root,
            &keep,
            &"b".repeat(32),
            b"keep me",
            long_ago,
        );
        with_rooms_handle(&rooms, |store| {
            store.close_with_marker(&cut, RoomCloser::Member("alice"), long_ago)?;
            store.close_with_marker(&keep, RoomCloser::Member("alice"), recently)?;
            Ok::<_, ocean_store::RoomStoreError>(())
        })
        .unwrap();

        let config = MaintenanceConfig {
            retention_days: 30,
            // Zero grace so the orphan half of the same sweep cannot be what
            // removed the cut room's blob: the retention path unlinks it by id.
            orphan_grace: Duration::ZERO,
            ..MaintenanceConfig::default()
        };
        let outcome = run_sweep(&rooms, &blob_root, &config, now);
        assert_eq!(outcome.error, None, "clean sweep");
        assert_eq!(outcome.rooms_cut, 1, "only the room past the window");

        // The cut room: transcript gone, attachment row gone, bytes gone. Its
        // `rooms` row survives, which is what keeps `/snapshot` able to say
        // `closed: true` instead of 404ing as a room that never existed.
        with_rooms_handle(&rooms, |store| {
            let page = store
                .transcript_page_including_closed(&cut, None, None)
                .expect("the room row still exists");
            assert!(page.messages.is_empty(), "cut room keeps no transcript");
            assert!(store
                .attachments(&cut)
                .expect("attachments read")
                .is_empty());
            assert!(store
                .get_including_closed(&cut)
                .expect("record read")
                .is_some());
        });
        assert!(
            !crate::room_attachments::room_dir(&blob_root, &cut)
                .join("a".repeat(32))
                .exists(),
            "the cut room's blob bytes must be unlinked after the commit"
        );

        // The room closed INSIDE the window keeps everything, bytes included.
        with_rooms_handle(&rooms, |store| {
            let page = store
                .transcript_page_including_closed(&keep, None, None)
                .unwrap();
            assert_eq!(
                page.messages.len(),
                4,
                "join marker, message, attachment marker and close marker all survive"
            );
            assert_eq!(store.attachments(&keep).unwrap().len(), 1);
        });
        assert_eq!(
            std::fs::read(
                crate::room_attachments::room_dir(&blob_root, &keep).join("b".repeat(32))
            )
            .unwrap(),
            b"keep me",
            "a room inside the window keeps its bytes byte-for-byte"
        );

        // The OPEN room is untouched and still open, however old it is.
        with_rooms_handle(&rooms, |store| {
            let record = store
                .get(&open)
                .unwrap()
                .expect("an open room is never eligible for retention");
            // Join marker plus the message. No attachment and no close marker.
            assert_eq!(record.transcript.len(), 2);
        });
    }

    /// Retention off — the default — cuts nothing at all.
    #[test]
    fn a_zero_window_is_never_and_cuts_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let (rooms, blob_root) = fixture(tmp.path());
        let now = DateTime::parse_from_rfc3339("2026-09-02T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let key = RoomKey::new("ancient");
        seed_room(&rooms, &key, now - chrono::Duration::days(4000));
        with_rooms_handle(&rooms, |store| {
            store.close_with_marker(
                &key,
                RoomCloser::Member("alice"),
                now - chrono::Duration::days(4000),
            )
        })
        .unwrap();

        let outcome = run_sweep(&rooms, &blob_root, &MaintenanceConfig::default(), now);
        assert_eq!(outcome.rooms_cut, 0);
        assert_eq!(outcome.messages_removed, 0);
        with_rooms_handle(&rooms, |store| {
            assert_eq!(
                store
                    .transcript_page_including_closed(&key, None, None)
                    .unwrap()
                    .messages
                    .len(),
                // Join marker, message, close marker: nothing was cut.
                3
            );
        });
    }

    #[test]
    fn orphan_gc_takes_the_unreferenced_blob_and_only_that_one() {
        let tmp = tempfile::tempdir().unwrap();
        let (rooms, blob_root) = fixture(tmp.path());
        let key = RoomKey::new("gc-room");
        seed_room(&rooms, &key, Utc::now());
        let live = "c".repeat(32);
        let orphan = "d".repeat(32);
        seed_attachment(
            &rooms,
            &blob_root,
            &key,
            &live,
            b"referenced bytes",
            Utc::now(),
        );
        crate::room_attachments::write_blob_for_test(&blob_root, &key, &orphan, b"orphan bytes");
        let config = MaintenanceConfig {
            orphan_grace: Duration::ZERO,
            ..MaintenanceConfig::default()
        };
        let outcome = run_sweep(&rooms, &blob_root, &config, Utc::now());
        assert_eq!(outcome.error, None);
        assert_eq!(outcome.orphan_files_removed, 1);
        assert_eq!(outcome.bytes_reclaimed, b"orphan bytes".len() as u64);
        let dir = crate::room_attachments::room_dir(&blob_root, &key);
        assert!(!dir.join(orphan).exists());
        assert_eq!(std::fs::read(dir.join(live)).unwrap(), b"referenced bytes");
    }

    /// A blob younger than the grace window survives.
    ///
    /// This is the correctness half of the grace, not caution: `room_attachments`
    /// fsyncs bytes BEFORE the row commits, so every successful upload spends a
    /// moment as an unreferenced file. A sweep with no grace races that window.
    #[test]
    fn a_blob_inside_the_grace_window_is_never_collected() {
        let tmp = tempfile::tempdir().unwrap();
        let (rooms, blob_root) = fixture(tmp.path());
        let key = RoomKey::new("grace-room");
        seed_room(&rooms, &key, Utc::now());
        let in_flight = "e".repeat(32);
        crate::room_attachments::write_blob_for_test(&blob_root, &key, &in_flight, b"mid-upload");

        let outcome = run_sweep(
            &rooms,
            &blob_root,
            &MaintenanceConfig::default(),
            Utc::now(),
        );
        assert_eq!(
            outcome.orphan_files_removed, 0,
            "an unreferenced blob inside the grace window is an upload in progress"
        );
        assert!(crate::room_attachments::room_dir(&blob_root, &key)
            .join(&in_flight)
            .exists());
    }

    /// A directory belonging to no room the store knows is collected whole —
    /// and a directory belonging to a CLOSED room is not.
    ///
    /// The closed half is the one worth a test. The blob path is a one-way hash
    /// of the room key, so the sweep can only ask "is this directory expected?"
    /// by re-deriving every room's; deriving only OPEN rooms would make every
    /// finished call's attachment directory unexpected, and the sweep would
    /// delete the files of every frozen room in the store.
    #[test]
    fn an_unknown_directory_goes_and_a_closed_rooms_directory_stays() {
        let tmp = tempfile::tempdir().unwrap();
        let (rooms, blob_root) = fixture(tmp.path());
        let now = Utc::now();
        let closed = RoomKey::new("frozen-room");
        seed_room(&rooms, &closed, now);
        seed_attachment(&rooms, &blob_root, &closed, &"f".repeat(32), b"frozen", now);
        with_rooms_handle(&rooms, |store| {
            store.close_with_marker(&closed, RoomCloser::Member("alice"), now)
        })
        .unwrap();

        // A directory under a hash of a key the store has never held.
        let stranger = crate::room_attachments::room_dir(&blob_root, &RoomKey::new("no-such-room"));
        std::fs::create_dir_all(&stranger).unwrap();
        std::fs::write(stranger.join("9".repeat(32)), b"nobody's").unwrap();

        let config = MaintenanceConfig {
            orphan_grace: Duration::ZERO,
            ..MaintenanceConfig::default()
        };
        let outcome = run_sweep(&rooms, &blob_root, &config, now);
        assert_eq!(outcome.error, None);
        assert_eq!(outcome.orphan_dirs_removed, 1);
        assert_eq!(outcome.bytes_reclaimed, b"nobody's".len() as u64);
        assert!(!stranger.exists(), "a directory no room claims is removed");
        assert_eq!(
            std::fs::read(
                crate::room_attachments::room_dir(&blob_root, &closed).join("f".repeat(32))
            )
            .unwrap(),
            b"frozen",
            "a soft-closed room still owns its files"
        );
    }

    #[test]
    fn orphan_gc_rechecks_new_room_and_attachment_after_enumeration() {
        let tmp = tempfile::tempdir().unwrap();
        let (rooms, root) = fixture(tmp.path());
        let key = RoomKey::new("adopted-after-listing");
        let id = "a".repeat(32);
        crate::room_attachments::write_blob_for_test(&root, &key, &id, b"keep");
        let mut outcome = SweepOutcome::default();
        run_orphan_gc_at(
            &rooms,
            &root,
            &MaintenanceConfig {
                orphan_grace: Duration::ZERO,
                ..MaintenanceConfig::default()
            },
            &mut outcome,
            std::time::SystemTime::now(),
            |_| {
                seed_room(&rooms, &key, Utc::now());
                with_rooms_handle(&rooms, |store| {
                    store.add_attachment(
                        &key,
                        &id,
                        "keep",
                        "text/plain",
                        4,
                        "digest",
                        "alice",
                        Utc::now(),
                    )
                })
                .unwrap();
            },
            |_| {},
        )
        .unwrap();
        assert_eq!(outcome.error, None);
        assert_eq!(outcome.bytes_reclaimed, 0);
        assert_eq!(outcome.orphan_dirs_removed, 0);
        assert_eq!(
            std::fs::read(crate::room_attachments::room_dir(&root, &key).join(id)).unwrap(),
            b"keep"
        );
    }

    #[test]
    fn orphan_gc_defers_an_old_inflight_publication_until_its_row_commits() {
        let tmp = tempfile::tempdir().unwrap();
        let (rooms, root) = fixture(tmp.path());
        let key = RoomKey::new("inflight");
        let id = "b".repeat(32);
        seed_room(&rooms, &key, Utc::now());
        let publication = crate::room_attachments::publication_guard();
        crate::room_attachments::write_blob_for_test(&root, &key, &id, b"old pending");
        let started = std::time::SystemTime::now();
        let file = crate::room_attachments::room_dir(&root, &key).join(&id);
        let old = started - Duration::from_secs(7200);
        std::fs::File::open(&file)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(old))
            .unwrap();
        let mut outcome = SweepOutcome::default();
        run_orphan_gc_at(
            &rooms,
            &root,
            &MaintenanceConfig::default(),
            &mut outcome,
            started,
            |_| {},
            |_| {},
        )
        .unwrap();
        assert_eq!(outcome.bytes_reclaimed, 0);
        assert_eq!(std::fs::read(&file).unwrap(), b"old pending");
        with_rooms_handle(&rooms, |store| {
            store.add_attachment(
                &key,
                &id,
                "pending",
                "text/plain",
                11,
                "digest",
                "alice",
                Utc::now(),
            )
        })
        .unwrap();
        drop(publication);
        run_orphan_gc_at(
            &rooms,
            &root,
            &MaintenanceConfig::default(),
            &mut outcome,
            started,
            |_| {},
            |_| {},
        )
        .unwrap();
        assert_eq!(outcome.bytes_reclaimed, 0);
        assert_eq!(std::fs::read(&file).unwrap(), b"old pending");
    }

    #[test]
    fn orphan_gc_cutoff_does_not_age_in_files_during_a_delayed_walk() {
        let tmp = tempfile::tempdir().unwrap();
        let (rooms, root) = fixture(tmp.path());
        let key = RoomKey::new("fixed-cutoff");
        seed_room(&rooms, &key, Utc::now());
        let id = "c".repeat(32);
        crate::room_attachments::write_blob_for_test(&root, &key, &id, b"new at start");
        // The file is old relative to the actual clock but was young relative
        // to this sweep's injected start. No sleeping or deletion-time clock.
        let started = std::time::SystemTime::now() - Duration::from_secs(7200);
        let file = crate::room_attachments::room_dir(&root, &key).join(&id);
        std::fs::File::open(&file)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(started))
            .unwrap();
        let mut outcome = SweepOutcome::default();
        run_orphan_gc_at(
            &rooms,
            &root,
            &MaintenanceConfig::default(),
            &mut outcome,
            started,
            |_| {},
            |_| {},
        )
        .unwrap();
        assert_eq!(outcome.bytes_reclaimed, 0);
        assert!(file.exists());
    }

    #[cfg(unix)]
    #[test]
    fn orphan_gc_uses_captured_directory_after_symlink_swap() {
        use std::os::unix::fs::symlink;
        let tmp = tempfile::tempdir().unwrap();
        let (rooms, root) = fixture(tmp.path());
        let key = RoomKey::new("swapped");
        seed_room(&rooms, &key, Utc::now());
        let id = "d".repeat(32);
        crate::room_attachments::write_blob_for_test(&root, &key, &id, b"orphan");
        let outside = tmp.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join(&id), b"sentinel").unwrap();
        let moved = tmp.path().join("captured");
        let mut outcome = SweepOutcome::default();
        run_orphan_gc_at(
            &rooms,
            &root,
            &MaintenanceConfig {
                orphan_grace: Duration::ZERO,
                ..MaintenanceConfig::default()
            },
            &mut outcome,
            std::time::SystemTime::now(),
            |_| {},
            |path| {
                std::fs::rename(path, &moved).unwrap();
                symlink(&outside, path).unwrap();
            },
        )
        .unwrap();
        assert_eq!(std::fs::read(outside.join(&id)).unwrap(), b"sentinel");
        assert!(!moved.join(&id).exists());
        assert_eq!(outcome.bytes_reclaimed, 6);
    }

    #[cfg(unix)]
    #[test]
    fn retention_refuses_replaced_directory_and_counts_actual_blob_length() {
        use std::os::unix::fs::symlink;
        let tmp = tempfile::tempdir().unwrap();
        let (rooms, root) = fixture(tmp.path());
        let now = Utc::now();
        let old = now - chrono::Duration::days(40);
        let key = RoomKey::new("retention-swap");
        seed_room(&rooms, &key, old);
        let id = "e".repeat(32);
        seed_attachment(&rooms, &root, &key, &id, b"actual", old);
        let dir = crate::room_attachments::room_dir(&root, &key);
        let outside = tmp.path().join("outside");
        std::fs::rename(&dir, &outside).unwrap();
        symlink(&outside, &dir).unwrap();
        with_rooms_handle(&rooms, |store| {
            store.close_with_marker(&key, RoomCloser::Member("alice"), old)
        })
        .unwrap();
        let config = MaintenanceConfig {
            retention_days: 30,
            ..MaintenanceConfig::default()
        };
        let mut outcome = SweepOutcome::default();
        run_retention(&rooms, &root, &config, now, &mut outcome).unwrap();
        assert_eq!(outcome.rooms_cut, 1);
        assert_eq!(outcome.bytes_reclaimed, 0);
        assert_eq!(outcome.blobs_unlink_failed, 1);
        assert_eq!(std::fs::read(outside.join(&id)).unwrap(), b"actual");
        // An ordinary blob changed since indexing is measured at unlink, not
        // credited using the stale metadata row's length.
        std::fs::remove_file(&dir).unwrap();
        let other = RoomKey::new("actual-size");
        seed_room(&rooms, &other, old);
        seed_attachment(&rooms, &root, &other, &id, b"short", old);
        std::fs::write(
            crate::room_attachments::room_dir(&root, &other).join(&id),
            b"larger replacement",
        )
        .unwrap();
        with_rooms_handle(&rooms, |store| {
            store.close_with_marker(&other, RoomCloser::Member("alice"), old)
        })
        .unwrap();
        let mut outcome = SweepOutcome::default();
        run_retention(&rooms, &root, &config, now, &mut outcome).unwrap();
        assert_eq!(outcome.bytes_reclaimed, b"larger replacement".len() as u64);
    }

    #[test]
    fn orphan_gc_leaves_nested_entries_instead_of_recursing() {
        let tmp = tempfile::tempdir().unwrap();
        let (rooms, root) = fixture(tmp.path());
        let dir = crate::room_attachments::room_dir(&root, &RoomKey::new("unknown"));
        std::fs::create_dir_all(dir.join("nested")).unwrap();
        std::fs::write(dir.join("nested/sentinel"), b"keep").unwrap();
        let outcome = run_sweep(
            &rooms,
            &root,
            &MaintenanceConfig {
                orphan_grace: Duration::ZERO,
                ..MaintenanceConfig::default()
            },
            Utc::now(),
        );
        assert_eq!(outcome.orphan_dirs_removed, 0);
        assert_eq!(outcome.bytes_reclaimed, 0);
        assert!(outcome.error.is_some());
        assert_eq!(std::fs::read(dir.join("nested/sentinel")).unwrap(), b"keep");
    }

    /// The window parse refuses to guess. Every unusable value is OFF, because
    /// the one safe direction to fail on a knob that deletes transcripts is
    /// "keep everything" — a typo must never become a SHORTER window.
    #[test]
    fn an_unusable_retention_value_is_off_and_never_a_guess() {
        assert_eq!(parse_retention_days("30"), 30);
        assert_eq!(parse_retention_days("  30  "), 30);
        assert_eq!(parse_retention_days("0"), 0);
        for unusable in [
            "",
            "   ",
            "thirty",
            "30d",
            "-1",
            "1.5",
            "99999999999999999999",
        ] {
            assert_eq!(
                parse_retention_days(unusable),
                DEFAULT_ROOM_RETENTION_DAYS,
                "{unusable:?} must leave retention off"
            );
        }
    }

    /// A recorded sweep is what `/health` reads, and the card carries the
    /// CONFIGURATION beside the counts.
    #[test]
    fn the_report_carries_the_configuration_and_the_last_runs_counts() {
        let config = MaintenanceConfig {
            retention_days: 45,
            orphan_grace: Duration::from_secs(60),
            interval: Duration::from_secs(3600),
        };
        let handle = new_handle(&config);
        let before = report_snapshot(&handle);
        assert_eq!(before.retention_days, 45);
        assert_eq!(before.interval_secs, 3600);
        assert_eq!(before.orphan_grace_secs, 60);
        assert_eq!(before.last_run_at, None, "no sweep has run yet");
        assert_eq!(before.runs_total, 0);

        record_sweep(
            &handle,
            &SweepOutcome {
                rooms_cut: 2,
                messages_removed: 17,
                bytes_reclaimed: 4096,
                error: Some("retention failed to cut a closed room".into()),
                ..SweepOutcome::default()
            },
            Utc::now(),
            Duration::from_millis(12),
        );
        let after = report_snapshot(&handle);
        assert_eq!(after.rooms_cut, 2);
        assert_eq!(after.messages_removed, 17);
        assert_eq!(after.bytes_reclaimed, 4096);
        assert_eq!(after.runs_total, 1);
        assert!(after.last_run_at.is_some());
        assert_eq!(
            after.last_error.as_deref(),
            Some("retention failed to cut a closed room"),
            "a sweep error must be readable on the card, not only in the log"
        );
        // Configuration survives a run: an operator reading `rooms_cut: 0` on a
        // later clean sweep can still see whether cutting is even on.
        assert_eq!(after.retention_days, 45);

        // A clean sweep CLEARS the error, so a non-null value always describes
        // the most recent run rather than the worst one ever seen.
        record_sweep(
            &handle,
            &SweepOutcome::default(),
            Utc::now(),
            Duration::from_millis(3),
        );
        let clean = report_snapshot(&handle);
        assert_eq!(clean.last_error, None);
        assert_eq!(clean.runs_total, 2);
    }

    /// A blob that cannot be unlinked is COUNTED and SURFACED, and its bytes
    /// are not claimed as reclaimed.
    ///
    /// This is the failure the whole operated half exists to catch, and it is
    /// invisible by construction: the rows commit, every count looks healthy,
    /// and disk simply never comes back. Discarding the `remove_file` error let
    /// a sweep report `last_error: null` and a `bytes_reclaimed` figure taken
    /// from the INDEX while the bytes were still on the filesystem — a report
    /// that is worse than none, because it actively says the opposite of what
    /// happened.
    ///
    /// The failure is induced by putting a DIRECTORY where the blob file should
    /// be, so `remove_file` fails with `EISDIR`. A read-only parent would not
    /// do: these tests can run as root, where mode bits are advisory and the
    /// unlink would succeed anyway.
    #[test]
    fn a_blob_that_cannot_be_unlinked_is_reported_not_swallowed() {
        let tmp = tempfile::tempdir().unwrap();
        let (rooms, blob_root) = fixture(tmp.path());
        let now = DateTime::parse_from_rfc3339("2026-09-02T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let long_ago = now - chrono::Duration::days(40);
        let key = RoomKey::new("unlinkable");
        seed_room(&rooms, &key, long_ago);

        // The row says there are 9 bytes; the path is a directory, so the
        // unlink cannot succeed.
        let id = "a".repeat(32);
        let stuck = crate::room_attachments::room_dir(&blob_root, &key).join(&id);
        std::fs::create_dir_all(&stuck).unwrap();
        with_rooms_handle(&rooms, |store| {
            store.add_attachment(
                &key,
                &id,
                "notes.txt",
                "text/plain",
                9,
                "0".repeat(64).as_str(),
                "alice",
                long_ago,
            )
        })
        .unwrap();
        with_rooms_handle(&rooms, |store| {
            store.close_with_marker(&key, RoomCloser::Member("alice"), long_ago)
        })
        .unwrap();

        let config = MaintenanceConfig {
            retention_days: 30,
            orphan_grace: Duration::ZERO,
            ..MaintenanceConfig::default()
        };
        let outcome = run_sweep(&rooms, &blob_root, &config, now);

        assert_eq!(outcome.rooms_cut, 1, "the row-level cut still happened");
        assert_eq!(
            outcome.blobs_unlink_failed, 1,
            "the failed unlink is counted, not discarded"
        );
        assert_eq!(
            outcome.bytes_reclaimed, 0,
            "bytes still on disk are never reported as reclaimed"
        );
        assert!(
            outcome.error.is_some(),
            "a sweep that could not free what it deleted is not a clean sweep"
        );
        assert!(stuck.exists(), "the fixture really did block the unlink");

        // The report an operator reads carries both.
        let handle = new_handle(&config);
        record_sweep(&handle, &outcome, now, Duration::from_millis(1));
        let card = report_snapshot(&handle);
        assert_eq!(card.blobs_unlink_failed, 1);
        assert_eq!(card.bytes_reclaimed, 0);
        assert!(card.last_error.is_some());
    }

    /// A corrupt/imported row is untrusted even though the HTTP upload path
    /// mints safe ids. Retention must never turn its stored id into an arbitrary
    /// filesystem path after the row-level cut has committed.
    #[test]
    fn a_malformed_stored_attachment_id_cannot_escape_the_blob_root() {
        let tmp = tempfile::tempdir().unwrap();
        let (rooms, blob_root) = fixture(tmp.path());
        let now = DateTime::parse_from_rfc3339("2026-09-02T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let long_ago = now - chrono::Duration::days(40);
        let key = RoomKey::new("malformed-retained-id");
        seed_room(&rooms, &key, long_ago);

        let outside = tmp.path().join("must-survive.txt");
        std::fs::write(&outside, b"outside").unwrap();
        let malformed = outside.to_string_lossy().into_owned();
        with_rooms_handle(&rooms, |store| {
            store.add_attachment(
                &key,
                &malformed,
                "notes.txt",
                "text/plain",
                7,
                "0".repeat(64).as_str(),
                "alice",
                long_ago,
            )
        })
        .unwrap();
        with_rooms_handle(&rooms, |store| {
            store.close_with_marker(&key, RoomCloser::Member("alice"), long_ago)
        })
        .unwrap();

        let config = MaintenanceConfig {
            retention_days: 30,
            orphan_grace: Duration::ZERO,
            ..MaintenanceConfig::default()
        };
        let outcome = run_sweep(&rooms, &blob_root, &config, now);

        assert_eq!(outcome.rooms_cut, 1);
        assert_eq!(outcome.blobs_unlink_failed, 1);
        assert_eq!(outcome.bytes_reclaimed, 0);
        assert_eq!(
            outcome.error.as_deref(),
            Some("retention refused a malformed stored attachment id")
        );
        assert_eq!(std::fs::read(&outside).unwrap(), b"outside");
    }

    /// A clean sweep still reports zero failures, so the counter above means
    /// something when it is nonzero.
    #[test]
    fn a_clean_sweep_reports_no_unlink_failures() {
        let tmp = tempfile::tempdir().unwrap();
        let (rooms, blob_root) = fixture(tmp.path());
        let now = DateTime::parse_from_rfc3339("2026-09-02T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let long_ago = now - chrono::Duration::days(40);
        let key = RoomKey::new("clean-cut");
        seed_room(&rooms, &key, long_ago);
        seed_attachment(
            &rooms,
            &blob_root,
            &key,
            &"b".repeat(32),
            b"12345678",
            long_ago,
        );
        with_rooms_handle(&rooms, |store| {
            store.close_with_marker(&key, RoomCloser::Member("alice"), long_ago)
        })
        .unwrap();

        let config = MaintenanceConfig {
            retention_days: 30,
            orphan_grace: Duration::ZERO,
            ..MaintenanceConfig::default()
        };
        let outcome = run_sweep(&rooms, &blob_root, &config, now);
        assert_eq!(outcome.blobs_unlink_failed, 0);
        assert_eq!(outcome.error, None);
        assert_eq!(
            outcome.bytes_reclaimed, 8,
            "bytes are claimed exactly when the file is actually gone"
        );
    }

    /// A second sweep over an already-cut archive does nothing and says so.
    ///
    /// The store's eligibility query is what makes this true; asserted from the
    /// sweep because that is where the misleading number would have shown up —
    /// every historical room recounted as another `rooms_cut`, on every run,
    /// forever.
    #[test]
    fn a_second_sweep_over_a_cut_archive_is_a_genuine_no_op() {
        let tmp = tempfile::tempdir().unwrap();
        let (rooms, blob_root) = fixture(tmp.path());
        let now = DateTime::parse_from_rfc3339("2026-09-02T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let long_ago = now - chrono::Duration::days(40);
        let key = RoomKey::new("archive");
        seed_room(&rooms, &key, long_ago);
        with_rooms_handle(&rooms, |store| {
            store.close_with_marker(&key, RoomCloser::Member("alice"), long_ago)
        })
        .unwrap();

        let config = MaintenanceConfig {
            retention_days: 30,
            orphan_grace: Duration::ZERO,
            ..MaintenanceConfig::default()
        };
        let first = run_sweep(&rooms, &blob_root, &config, now);
        assert_eq!(first.rooms_cut, 1);
        assert!(first.messages_removed > 0);

        let second = run_sweep(&rooms, &blob_root, &config, now);
        assert_eq!(
            second.rooms_cut, 0,
            "an emptied room must not be recounted on every future sweep"
        );
        assert_eq!(second.messages_removed, 0);
        assert_eq!(second.error, None);
    }
}
