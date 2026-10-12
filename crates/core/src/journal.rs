//! The recovery journal: an append-only JSONL file of every change Vigilia makes to the OS.
//!
//! Every action follows the same path:
//! 1. append `intent` and fsync, **before** touching the OS;
//! 2. apply the change, then append `applied` (or `failed`, which closes the action);
//! 3. later, undo the change and append `undone` (or `failed`, which leaves it open).
//!
//! After a crash, [`Journal::replay`] undoes every action that was not closed. When nothing is
//! open the file is truncated, so between sessions it is empty.

use std::collections::BTreeMap;
use std::fmt::Display;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The kinds of OS changes Vigilia makes. Later steps add variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    KeepAwake,
}

/// The state needed to undo one action. Later steps add variants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UndoRecord {
    /// An IOKit `PreventUserIdleSystemSleep` assertion. The OS releases it when the owning
    /// process exits, so after a crash there is nothing left to undo.
    IdleSleepAssertion { pid: u32, assertion_id: u32 },
    /// A backend that holds no OS state (tests, stubs).
    Noop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Intent,
    Applied,
    Undone,
    Failed,
}

/// Identifies one action: the `seq` of its `intent` entry.
pub type ActionId = u64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub seq: u64,
    pub ts: DateTime<Utc>,
    pub action_id: ActionId,
    pub action: ActionKind,
    pub phase: Phase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undo: Option<UndoRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// An action that was started and not yet closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingAction {
    pub id: ActionId,
    pub kind: ActionKind,
    /// `false` when the process stopped between `intent` and `applied`: the change may or may
    /// not have happened, and undo must cope with both.
    pub applied: bool,
    pub undo: Option<UndoRecord>,
}

#[derive(Debug, thiserror::Error)]
pub enum ActionError<E> {
    #[error("journal write failed: {0}")]
    Journal(#[from] io::Error),
    #[error("{0}")]
    Os(E),
}

#[derive(Debug, Default)]
pub struct ReplayReport {
    pub undone: Vec<PendingAction>,
    pub failed: Vec<(PendingAction, String)>,
}

#[derive(Debug)]
pub struct Journal {
    path: PathBuf,
    file: File,
    next_seq: u64,
    open: BTreeMap<ActionId, PendingAction>,
}

impl Journal {
    /// Opens or creates the journal and reads every entry. A torn last line (a crash during a
    /// write) is dropped and cut from the file.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        // Plain write access rather than append mode: Windows refuses `set_len` on append-only
        // handles. We are the only writer, so writing at the end we track is equivalent.
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;

        let mut journal = Self {
            path,
            file,
            next_seq: 1,
            open: BTreeMap::new(),
        };
        let mut good_len = 0;
        let mut rest = bytes.as_slice();
        while !rest.is_empty() {
            let Some(end) = rest.iter().position(|&b| b == b'\n') else {
                tracing::warn!(bytes = rest.len(), "dropping torn last journal line");
                break;
            };
            let line = &rest[..end];
            match serde_json::from_slice::<Entry>(line) {
                Ok(entry) => journal.track(&entry),
                Err(e) if !line.is_empty() => {
                    tracing::warn!(error = %e, "skipping unreadable journal line");
                }
                Err(_) => {}
            }
            good_len += end + 1;
            rest = &rest[end + 1..];
        }
        if good_len < bytes.len() {
            journal.file.set_len(good_len as u64)?;
            journal.file.sync_data()?;
        }
        journal.file.seek(SeekFrom::Start(good_len as u64))?;
        Ok(journal)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Actions that were started and not closed, oldest first.
    pub fn pending(&self) -> Vec<PendingAction> {
        self.open.values().cloned().collect()
    }

    /// Runs `apply` between an `intent` entry (fsynced first) and an `applied` entry.
    /// `planned` is the undo record if it is known before applying.
    pub fn apply<E: Display>(
        &mut self,
        kind: ActionKind,
        planned: Option<UndoRecord>,
        apply: impl FnOnce() -> Result<UndoRecord, E>,
    ) -> Result<(ActionId, UndoRecord), ActionError<E>> {
        let id = self.next_seq;
        self.append(id, kind, Phase::Intent, planned, None)?;
        match apply() {
            Ok(undo) => {
                self.append(id, kind, Phase::Applied, Some(undo.clone()), None)?;
                Ok((id, undo))
            }
            Err(e) => {
                self.append(id, kind, Phase::Failed, None, Some(e.to_string()))?;
                Err(ActionError::Os(e))
            }
        }
    }

    /// Runs `undo` for an open action and closes it on success. On failure the action stays
    /// open, so a later retry or [`replay`](Self::replay) tries again.
    pub fn undo<E: Display>(
        &mut self,
        id: ActionId,
        undo: impl FnOnce() -> Result<(), E>,
    ) -> Result<(), ActionError<E>> {
        let Some(kind) = self.open.get(&id).map(|a| a.kind) else {
            return Ok(());
        };
        match undo() {
            Ok(()) => {
                self.append(id, kind, Phase::Undone, None, None)?;
                self.compact_if_clean()?;
                Ok(())
            }
            Err(e) => {
                self.append(id, kind, Phase::Failed, None, Some(e.to_string()))?;
                Err(ActionError::Os(e))
            }
        }
    }

    /// Undoes every open action, oldest first, using `restore`. Used at startup and by
    /// `vigilia recover`.
    pub fn replay<E: Display>(
        &mut self,
        mut restore: impl FnMut(&PendingAction) -> Result<(), E>,
    ) -> io::Result<ReplayReport> {
        let mut report = ReplayReport::default();
        for action in self.pending() {
            match restore(&action) {
                Ok(()) => {
                    self.append(
                        action.id,
                        action.kind,
                        Phase::Undone,
                        None,
                        Some("replayed".into()),
                    )?;
                    report.undone.push(action);
                }
                Err(e) => {
                    let detail = e.to_string();
                    self.append(
                        action.id,
                        action.kind,
                        Phase::Failed,
                        None,
                        Some(detail.clone()),
                    )?;
                    report.failed.push((action, detail));
                }
            }
        }
        self.compact_if_clean()?;
        Ok(report)
    }

    fn append(
        &mut self,
        action_id: ActionId,
        action: ActionKind,
        phase: Phase,
        undo: Option<UndoRecord>,
        detail: Option<String>,
    ) -> io::Result<()> {
        let entry = Entry {
            seq: self.next_seq,
            ts: Utc::now(),
            action_id,
            action,
            phase,
            undo,
            detail,
        };
        let mut line = serde_json::to_vec(&entry).map_err(io::Error::other)?;
        line.push(b'\n');
        self.file.write_all(&line)?;
        self.file.sync_data()?;
        self.track(&entry);
        Ok(())
    }

    fn track(&mut self, entry: &Entry) {
        self.next_seq = self.next_seq.max(entry.seq + 1);
        match entry.phase {
            Phase::Intent => {
                self.open.insert(
                    entry.action_id,
                    PendingAction {
                        id: entry.action_id,
                        kind: entry.action,
                        applied: false,
                        undo: entry.undo.clone(),
                    },
                );
            }
            Phase::Applied => {
                if let Some(a) = self.open.get_mut(&entry.action_id) {
                    a.applied = true;
                    if entry.undo.is_some() {
                        a.undo = entry.undo.clone();
                    }
                }
            }
            Phase::Undone => {
                self.open.remove(&entry.action_id);
            }
            Phase::Failed => {
                // A failed apply closes the action; a failed undo leaves it open.
                if self.open.get(&entry.action_id).is_some_and(|a| !a.applied) {
                    self.open.remove(&entry.action_id);
                }
            }
        }
    }

    fn compact_if_clean(&mut self) -> io::Result<()> {
        if self.open.is_empty() && self.file.metadata()?.len() > 0 {
            self.file.set_len(0)?;
            self.file.seek(SeekFrom::Start(0))?;
            self.file.sync_data()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::convert::Infallible;

    fn record(n: u32) -> UndoRecord {
        UndoRecord::IdleSleepAssertion {
            pid: 1,
            assertion_id: n,
        }
    }

    fn id_of(rec: &UndoRecord) -> u32 {
        match rec {
            UndoRecord::IdleSleepAssertion { assertion_id, .. } => *assertion_id,
            UndoRecord::Noop => 0,
        }
    }

    #[test]
    fn apply_then_undo_leaves_an_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.jsonl");
        let mut j = Journal::open(&path).unwrap();
        let (id, undo) = j
            .apply(ActionKind::KeepAwake, None, || {
                Ok::<_, Infallible>(record(7))
            })
            .unwrap();
        assert_eq!(undo, record(7));
        assert_eq!(j.pending().len(), 1);
        assert!(std::fs::read_to_string(&path).unwrap().lines().count() == 2);

        j.undo(id, || Ok::<_, Infallible>(())).unwrap();
        assert!(j.pending().is_empty());
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);

        // After compaction, new entries start at the beginning of the file again.
        j.apply(ActionKind::KeepAwake, None, || {
            Ok::<_, Infallible>(record(8))
        })
        .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 2, "{text:?}");
        assert!(!text.starts_with('\0'));
        assert_eq!(Journal::open(&path).unwrap().pending().len(), 1);
    }

    #[test]
    fn failed_apply_closes_the_action_and_failed_undo_keeps_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut j = Journal::open(dir.path().join("j")).unwrap();
        let err = j
            .apply(ActionKind::KeepAwake, None, || {
                Err::<UndoRecord, _>("denied")
            })
            .unwrap_err();
        assert!(matches!(err, ActionError::Os("denied")));
        assert!(j.pending().is_empty());

        let (id, _) = j
            .apply(ActionKind::KeepAwake, None, || {
                Ok::<_, Infallible>(record(1))
            })
            .unwrap();
        assert!(j.undo(id, || Err("busy")).is_err());
        let reopened = Journal::open(j.path()).unwrap();
        assert_eq!(reopened.pending().len(), 1);
        assert!(reopened.pending()[0].applied);
    }

    #[test]
    fn torn_last_line_is_dropped_and_cut() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("j");
        let mut j = Journal::open(&path).unwrap();
        j.apply(ActionKind::KeepAwake, None, || {
            Ok::<_, Infallible>(record(1))
        })
        .unwrap();
        drop(j);
        let good_len = std::fs::metadata(&path).unwrap().len();
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(br#"{"seq":3,"ts":"2026-"#)
            .unwrap();

        let mut j = Journal::open(&path).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), good_len);
        assert_eq!(j.pending().len(), 1);
        let report = j.replay(|_| Ok::<_, Infallible>(())).unwrap();
        assert_eq!(report.undone.len(), 1);
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
    }

    /// Where a simulated crash stops an operation.
    #[derive(Debug, Clone, Copy)]
    enum Stage {
        BeforeIntent,
        AfterIntent,
        AfterOsChange,
    }

    #[derive(Debug, Clone)]
    enum Op {
        Apply,
        /// Undo the open action at this index (modulo the number open).
        Undo(usize),
    }

    fn op() -> impl proptest::strategy::Strategy<Value = Op> {
        use proptest::prelude::*;
        prop_oneof![Just(Op::Apply), any::<usize>().prop_map(Op::Undo)]
    }

    fn stage() -> impl proptest::strategy::Strategy<Value = Stage> {
        use proptest::prelude::*;
        prop_oneof![
            Just(Stage::BeforeIntent),
            Just(Stage::AfterIntent),
            Just(Stage::AfterOsChange)
        ]
    }

    proptest::proptest! {
        /// Run some actions, crash part-way through the next one, reopen and replay. Whatever
        /// the crash point, the fake OS must end with nothing applied, every open action must be
        /// restored exactly once, and the journal must end empty.
        #[test]
        fn replay_after_a_crash_at_any_step_leaves_a_clean_state(
            ops in proptest::collection::vec(op(), 0..12),
            crash_op in op(),
            stage in stage(),
        ) {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("journal.jsonl");
            let mut os: BTreeSet<u32> = BTreeSet::new();
            let mut next = 1u32;
            let mut open: Vec<(ActionId, u32)> = Vec::new();
            let mut j = Journal::open(&path).unwrap();

            for op in &ops {
                match op {
                    Op::Apply => {
                        let n = next;
                        next += 1;
                        let (id, _) = j
                            .apply(ActionKind::KeepAwake, Some(record(n)), || {
                                os.insert(n);
                                Ok::<_, Infallible>(record(n))
                            })
                            .unwrap();
                        open.push((id, n));
                    }
                    Op::Undo(i) if !open.is_empty() => {
                        let (id, n) = open.remove(i % open.len());
                        j.undo(id, || {
                            os.remove(&n);
                            Ok::<_, Infallible>(())
                        })
                        .unwrap();
                    }
                    Op::Undo(_) => {}
                }
            }

            // The crash: perform only part of `crash_op`, then drop everything in memory.
            match (crash_op, stage) {
                (_, Stage::BeforeIntent) => {}
                (Op::Apply, Stage::AfterIntent | Stage::AfterOsChange) => {
                    let n = next;
                    let id = j.next_seq;
                    j.append(id, ActionKind::KeepAwake, Phase::Intent, Some(record(n)), None).unwrap();
                    if matches!(stage, Stage::AfterOsChange) {
                        os.insert(n);
                    }
                }
                (Op::Undo(i), Stage::AfterIntent | Stage::AfterOsChange) if !open.is_empty() => {
                    // Undo has no intent entry: the OS change happened, the `undone` line didn't.
                    let (_, n) = open[i % open.len()];
                    os.remove(&n);
                }
                (Op::Undo(_), _) => {}
            }
            drop(j);

            let mut j = Journal::open(&path).unwrap();
            let pending = j.pending();
            let mut restored = Vec::new();
            let report = j
                .replay(|a| {
                    restored.push(a.id);
                    let rec = a.undo.as_ref().expect("planned undo record is journaled");
                    os.remove(&id_of(rec));
                    Ok::<_, Infallible>(())
                })
                .unwrap();

            proptest::prop_assert!(os.is_empty(), "left applied: {os:?}");
            proptest::prop_assert_eq!(restored, pending.iter().map(|a| a.id).collect::<Vec<_>>());
            proptest::prop_assert!(report.failed.is_empty());
            proptest::prop_assert!(j.pending().is_empty());
            proptest::prop_assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
        }
    }
}
