use std::{
    fmt,
    fs::File,
    io::Read,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use rusqlite::{params, Connection};
use serde::Serialize;
use walkdir::WalkDir;

pub const MAX_BYTES: u64 = 1_048_576;
const MAX_ENTRIES: usize = 100_000;
const SCAN_BUDGET: Duration = Duration::from_secs(300);

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ActiveIndex {
    pub generation: u64,
    pub documents: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    SourceUnavailable,
    Storage,
    ReadErrors,
    ScanLimit,
}

#[derive(Clone, Default, Serialize)]
pub struct Progress {
    pub phase: String,
    pub found: usize,
    pub processed: usize,
    pub skipped: usize,
    pub errors: usize,
    // Retained for older clients. Diagnostics use fixed reason codes, not file contents/paths.
    pub message: String,
    pub reason: Option<Reason>,
    pub active: ActiveIndex,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ScanOutcome {
    Applied(ActiveIndex),
    Retained { active: ActiveIndex, reason: Reason },
}

#[derive(Debug)]
pub enum ScanError {
    SourceUnavailable,
    Storage(rusqlite::Error),
}

impl fmt::Display for ScanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::SourceUnavailable => "source_unavailable",
            Self::Storage(_) => "storage",
        })
    }
}

impl std::error::Error for ScanError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Storage(error) => Some(error),
            Self::SourceUnavailable => None,
        }
    }
}

impl From<rusqlite::Error> for ScanError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Storage(error)
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ReadFailure {
    Unreadable,
    TooLarge,
    InvalidUtf8,
}

/// Read at most the limit plus one sentinel byte, even if a file grows after metadata().
fn bounded_text(reader: impl Read) -> Result<String, ReadFailure> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ReadFailure::Unreadable)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(ReadFailure::TooLarge);
    }
    String::from_utf8(bytes).map_err(|_| ReadFailure::InvalidUtf8)
}

fn read_document(path: &Path) -> Result<String, ReadFailure> {
    let file = File::open(path).map_err(|_| ReadFailure::Unreadable)?;
    let metadata = file.metadata().map_err(|_| ReadFailure::Unreadable)?;
    if !metadata.is_file() {
        return Err(ReadFailure::Unreadable);
    }
    if metadata.len() > MAX_BYTES {
        return Err(ReadFailure::TooLarge);
    }
    bounded_text(file)
}

pub fn database(path: &Path) -> rusqlite::Result<Connection> {
    let connection = Connection::open(path)?;
    connection.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE VIRTUAL TABLE IF NOT EXISTS docs USING fts5(
             path UNINDEXED,title,body,tokenize='unicode61'
         );
         CREATE TABLE IF NOT EXISTS pocket_index_state(
             singleton INTEGER PRIMARY KEY CHECK(singleton=1),
             generation INTEGER NOT NULL CHECK(typeof(generation)='integer' AND generation>=0)
         );
         INSERT OR IGNORE INTO pocket_index_state(singleton,generation) VALUES(1,0);",
    )?;
    Ok(connection)
}

pub fn active_index(connection: &Connection) -> rusqlite::Result<ActiveIndex> {
    connection.query_row(
        "SELECT generation,(SELECT count(*) FROM docs) FROM pocket_index_state WHERE singleton=1",
        [],
        |row| {
            Ok(ActiveIndex {
                generation: row.get(0)?,
                documents: row.get(1)?,
            })
        },
    )
}

pub fn set_progress(state: &Arc<Mutex<Progress>>, update: impl FnOnce(&mut Progress)) {
    if let Ok(mut progress) = state.lock() {
        update(&mut progress);
    }
}

/// Publish a complete snapshot only; unsupported extensions and symlinks are intentional skips.
pub fn scan(
    root: &Path,
    path: &Path,
    progress: &Arc<Mutex<Progress>>,
) -> Result<ScanOutcome, ScanError> {
    scan_with_reader(root, path, progress, read_document)
}

// A narrow file-reader boundary lets tests pause a real SQLite transaction or inject a read failure.
fn scan_with_reader(
    root: &Path,
    path: &Path,
    progress: &Arc<Mutex<Progress>>,
    read: impl FnMut(&Path) -> Result<String, ReadFailure>,
) -> Result<ScanOutcome, ScanError> {
    scan_with_limits(root, path, progress, read, MAX_ENTRIES, SCAN_BUDGET)
}

fn scan_with_limits(
    root: &Path,
    path: &Path,
    progress: &Arc<Mutex<Progress>>,
    mut read: impl FnMut(&Path) -> Result<String, ReadFailure>,
    max_entries: usize,
    budget: Duration,
) -> Result<ScanOutcome, ScanError> {
    let started = Instant::now();
    let root = root
        .canonicalize()
        .map_err(|_| ScanError::SourceUnavailable)?;
    if !root.is_dir() {
        return Err(ScanError::SourceUnavailable);
    }
    let mut connection = database(path)?;
    let previous = active_index(&connection)?;
    set_progress(progress, |p| p.active = previous.clone());
    let transaction = connection.transaction()?;
    transaction.execute("DELETE FROM docs", [])?;
    let mut failures = 0;
    let mut retained_reason = Reason::ReadErrors;
    for (visited, entry) in WalkDir::new(&root)
        .follow_links(false)
        .into_iter()
        .enumerate()
    {
        if visited >= max_entries || started.elapsed() > budget {
            failures += 1;
            retained_reason = Reason::ScanLimit;
            set_progress(progress, |p| p.errors += 1);
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                failures += 1;
                set_progress(progress, |p| p.errors += 1);
                continue;
            }
        };
        if entry.file_type().is_symlink() {
            set_progress(progress, |p| p.skipped += 1);
            continue;
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let extension = entry
            .path()
            .extension()
            .and_then(|x| x.to_str())
            .unwrap_or("");
        if !extension.eq_ignore_ascii_case("md") && !extension.eq_ignore_ascii_case("txt") {
            set_progress(progress, |p| p.skipped += 1);
            continue;
        }
        set_progress(progress, |p| p.found += 1);
        let text = match read(entry.path()) {
            Ok(text) => text,
            Err(_) => {
                failures += 1;
                set_progress(progress, |p| p.errors += 1);
                continue;
            }
        };
        let relative = entry
            .path()
            .strip_prefix(&root)
            .map_err(|_| ScanError::SourceUnavailable)?;
        transaction.execute(
            "INSERT INTO docs(path,title,body) VALUES(?1,?2,?3)",
            params![
                relative.to_string_lossy(),
                entry.file_name().to_string_lossy(),
                text
            ],
        )?;
        set_progress(progress, |p| p.processed += 1);
    }
    if failures == 0 && started.elapsed() > budget {
        failures += 1;
        retained_reason = Reason::ScanLimit;
        set_progress(progress, |p| p.errors += 1);
    }
    if failures > 0 {
        transaction.rollback()?;
        return Ok(ScanOutcome::Retained {
            active: previous,
            reason: retained_reason,
        });
    }
    // Metadata and FTS rows commit together; a failed generation write rolls the snapshot back.
    transaction.execute(
        "UPDATE pocket_index_state SET generation=generation+1 WHERE singleton=1",
        [],
    )?;
    let active = active_index(&transaction)?;
    transaction.commit()?;
    Ok(ScanOutcome::Applied(active))
}

/// Update the public status only after the transaction has committed or rolled back.
pub fn run_scan(root: &Path, path: &Path, progress: &Arc<Mutex<Progress>>) {
    let result = scan(root, path, progress);
    set_progress(progress, |p| match result {
        Ok(ScanOutcome::Applied(active)) => {
            p.active = active;
            p.phase = "complete".into();
            p.reason = None;
        }
        Ok(ScanOutcome::Retained { active, reason }) => {
            p.active = active;
            p.phase = "retained".into();
            p.reason = Some(reason);
        }
        Err(error) => {
            // Do not expose file paths, note text, SQL or raw operating-system error details.
            p.phase = "error".into();
            p.reason = Some(match error {
                ScanError::SourceUnavailable => Reason::SourceUnavailable,
                ScanError::Storage(_) => Reason::Storage,
            });
            p.errors += 1;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io, sync::mpsc, thread};

    fn state() -> Arc<Mutex<Progress>> {
        Arc::new(Mutex::new(Progress::default()))
    }
    fn matches(path: &Path, term: &str) -> rusqlite::Result<usize> {
        Connection::open(path)?.query_row(
            "SELECT count(*) FROM docs WHERE docs MATCH ?1",
            [term],
            |r| r.get(0),
        )
    }

    #[test]
    fn invalid_utf8_keeps_snapshot_and_successful_retry_replaces_it() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("notes");
        std::fs::create_dir(&root)?;
        let db = temp.path().join("index.sqlite");
        std::fs::write(root.join("note.md"), "oldtoken")?;
        scan(&root, &db, &state())?;
        let before = active_index(&database(&db)?)?;
        std::fs::write(root.join("note.md"), "newtoken")?;
        std::fs::write(root.join("broken.txt"), [255, 254])?;
        let progress = state();
        run_scan(&root, &db, &progress);
        assert_eq!(
            progress.lock().map_err(|_| io::Error::other("lock"))?.phase,
            "retained"
        );
        assert_eq!(active_index(&database(&db)?)?, before);
        assert_eq!(matches(&db, "oldtoken")?, 1);
        assert_eq!(matches(&db, "newtoken")?, 0);
        std::fs::remove_file(root.join("broken.txt"))?;
        scan(&root, &db, &state())?;
        assert_eq!(matches(&db, "oldtoken")?, 0);
        assert_eq!(matches(&db, "newtoken")?, 1);
        assert_eq!(
            active_index(&database(&db)?)?.generation,
            before.generation + 1
        );
        Ok(())
    }

    #[test]
    fn actual_read_limit_does_not_trust_metadata_or_unbounded_growth() {
        struct Growing {
            read: u64,
        }
        impl Read for Growing {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                buffer.fill(b'x');
                self.read += buffer.len() as u64;
                Ok(buffer.len())
            }
        }
        let mut source = Growing { read: 0 };
        assert_eq!(bounded_text(&mut source), Err(ReadFailure::TooLarge));
        assert_eq!(source.read, MAX_BYTES + 1);
        assert!(bounded_text(io::repeat(b'x').take(MAX_BYTES)).is_ok());
    }

    #[test]
    fn oversized_candidate_retains_previous_index() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("notes");
        std::fs::create_dir(&root)?;
        let db = temp.path().join("index.sqlite");
        std::fs::write(root.join("one.txt"), "oldtoken")?;
        scan(&root, &db, &state())?;
        std::fs::write(root.join("one.txt"), vec![b'x'; MAX_BYTES as usize + 1])?;
        assert!(matches!(
            scan(&root, &db, &state())?,
            ScanOutcome::Retained { .. }
        ));
        assert_eq!(matches(&db, "oldtoken")?, 1);
        Ok(())
    }

    #[test]
    fn sql_failure_rolls_back_fts_and_generation() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("notes");
        std::fs::create_dir(&root)?;
        let db = temp.path().join("index.sqlite");
        std::fs::write(root.join("one.md"), "oldtoken")?;
        scan(&root, &db, &state())?;
        let before = active_index(&database(&db)?)?;
        database(&db)?.execute_batch("CREATE TRIGGER reject_snapshot BEFORE UPDATE ON pocket_index_state BEGIN SELECT RAISE(ABORT,'fixture'); END;")?;
        std::fs::write(root.join("one.md"), "newtoken")?;
        assert!(matches!(
            scan(&root, &db, &state()),
            Err(ScanError::Storage(_))
        ));
        assert_eq!(active_index(&database(&db)?)?, before);
        assert_eq!(matches(&db, "oldtoken")?, 1);
        assert_eq!(matches(&db, "newtoken")?, 0);
        Ok(())
    }

    #[test]
    fn readers_see_previous_snapshot_while_rebuild_is_uncommitted() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("notes");
        std::fs::create_dir(&root)?;
        let db = temp.path().join("index.sqlite");
        std::fs::write(root.join("one.md"), "oldtoken")?;
        scan(&root, &db, &state())?;
        std::fs::write(root.join("one.md"), "newtoken")?;
        std::fs::write(root.join("two.txt"), "newtoken")?;
        let (paused_tx, paused_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        let thread_root = root.clone();
        let thread_db = db.clone();
        let worker = thread::spawn(move || {
            let mut calls = 0;
            scan_with_reader(&thread_root, &thread_db, &state(), |path| {
                calls += 1;
                if calls == 2 {
                    paused_tx.send(()).map_err(|_| ReadFailure::Unreadable)?;
                    resume_rx.recv().map_err(|_| ReadFailure::Unreadable)?;
                    return Err(ReadFailure::Unreadable);
                }
                read_document(path)
            })
        });
        paused_rx.recv_timeout(std::time::Duration::from_secs(5))?;
        let old = matches(&db, "oldtoken");
        let new = matches(&db, "newtoken");
        resume_tx.send(())?;
        assert!(matches!(
            worker.join().map_err(|_| io::Error::other("worker"))??,
            ScanOutcome::Retained { .. }
        ));
        assert_eq!(old?, 1);
        assert_eq!(new?, 0);
        assert_eq!(matches(&db, "oldtoken")?, 1);
        Ok(())
    }

    #[test]
    fn entry_and_cooperative_time_limits_do_not_publish_partial_data() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("notes");
        std::fs::create_dir(&root)?;
        let db = temp.path().join("index.sqlite");
        std::fs::write(root.join("one.md"), "oldtoken")?;
        scan(&root, &db, &state())?;
        std::fs::write(root.join("one.md"), "newtoken")?;
        let limited = scan_with_limits(&root, &db, &state(), read_document, 1, SCAN_BUDGET)?;
        assert!(matches!(
            limited,
            ScanOutcome::Retained {
                reason: Reason::ScanLimit,
                ..
            }
        ));
        let slow = scan_with_limits(
            &root,
            &db,
            &state(),
            |path| {
                thread::sleep(Duration::from_millis(30));
                read_document(path)
            },
            MAX_ENTRIES,
            Duration::from_millis(20),
        )?;
        assert!(matches!(
            slow,
            ScanOutcome::Retained {
                reason: Reason::ScanLimit,
                ..
            }
        ));
        assert_eq!(matches(&db, "oldtoken")?, 1);
        assert_eq!(matches(&db, "newtoken")?, 0);
        Ok(())
    }

    #[test]
    fn legacy_index_survives_metadata_upgrade_and_missing_root() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let db = temp.path().join("legacy.sqlite");
        let connection = Connection::open(&db)?;
        connection.execute_batch("CREATE VIRTUAL TABLE docs USING fts5(path UNINDEXED,title,body,tokenize='unicode61'); INSERT INTO docs VALUES('a.md','a.md','oldtoken');")?;
        drop(connection);
        let active = active_index(&database(&db)?)?;
        assert_eq!(active.documents, 1);
        assert_eq!(active.generation, 0);
        assert!(matches!(
            scan(&temp.path().join("missing"), &db, &state()),
            Err(ScanError::SourceUnavailable)
        ));
        assert_eq!(matches(&db, "oldtoken")?, 1);
        Ok(())
    }

    #[test]
    fn complete_empty_folder_is_a_valid_snapshot_and_first_failure_is_empty() -> anyhow::Result<()>
    {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("notes");
        std::fs::create_dir(&root)?;
        let db = temp.path().join("index.sqlite");
        std::fs::write(root.join("bad.md"), [255])?;
        assert!(matches!(
            scan(&root, &db, &state())?,
            ScanOutcome::Retained {
                active: ActiveIndex { documents: 0, .. },
                ..
            }
        ));
        std::fs::remove_file(root.join("bad.md"))?;
        let result = scan(&root, &db, &state())?;
        assert_eq!(
            result,
            ScanOutcome::Applied(ActiveIndex {
                documents: 0,
                generation: 1
            })
        );
        Ok(())
    }
}
