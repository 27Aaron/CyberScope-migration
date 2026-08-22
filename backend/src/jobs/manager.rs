use std::{
    collections::HashMap,
    sync::{Arc, Mutex, RwLock},
    time::SystemTime,
};

use thiserror::Error;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobKind {
    Search,
    Batch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobStatus {
    Queued,
    Running,
    Cancelling,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug)]
pub struct JobSnapshot {
    pub id: Uuid,
    pub user_id: u64,
    pub kind: JobKind,
    pub status: JobStatus,
    pub started_at: SystemTime,
    pub committed_batches: u64,
    pub upstream_attempts: u64,
    pub written_rows: u64,
    pub failed_queries: u64,
    pub retries: u64,
    pub possible_duplicate_charge: bool,
}

struct JobEntry {
    snapshot: RwLock<JobSnapshot>,
    cancellation: CancellationToken,
}

pub struct JobManager {
    jobs: Mutex<HashMap<u64, Arc<JobEntry>>>,
    slots: Arc<Semaphore>,
}

impl JobManager {
    pub fn new(max_concurrent_jobs: usize) -> Arc<Self> {
        Arc::new(Self {
            jobs: Mutex::new(HashMap::new()),
            slots: Arc::new(Semaphore::new(max_concurrent_jobs)),
        })
    }

    pub fn try_start(
        self: &Arc<Self>,
        user_id: u64,
        kind: JobKind,
    ) -> Result<JobLease, JobStartError> {
        {
            let jobs = self.jobs.lock().expect("jobs mutex poisoned");
            if jobs.contains_key(&user_id) {
                return Err(JobStartError::UserBusy);
            }
        }

        let permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| JobStartError::GloballyBusy)?;

        let id = Uuid::new_v4();
        let cancellation = CancellationToken::new();
        let entry = Arc::new(JobEntry {
            snapshot: RwLock::new(JobSnapshot {
                id,
                user_id,
                kind,
                status: JobStatus::Queued,
                started_at: SystemTime::now(),
                committed_batches: 0,
                upstream_attempts: 0,
                written_rows: 0,
                failed_queries: 0,
                retries: 0,
                possible_duplicate_charge: false,
            }),
            cancellation: cancellation.clone(),
        });

        let mut jobs = self.jobs.lock().expect("jobs mutex poisoned");
        if jobs.contains_key(&user_id) {
            return Err(JobStartError::UserBusy);
        }
        jobs.insert(user_id, entry.clone());

        Ok(JobLease {
            manager: self.clone(),
            entry,
            permit: Some(permit),
            id,
            user_id,
        })
    }

    pub fn snapshot(&self, user_id: u64) -> Option<JobSnapshot> {
        let entry = self
            .jobs
            .lock()
            .expect("jobs mutex poisoned")
            .get(&user_id)
            .cloned()?;
        let snapshot = entry
            .snapshot
            .read()
            .expect("job snapshot lock poisoned")
            .clone();
        Some(snapshot)
    }

    pub fn snapshot_by_id(&self, id: Uuid) -> Option<JobSnapshot> {
        let entries = self.jobs.lock().expect("jobs mutex poisoned");
        entries.values().find_map(|entry| {
            let snapshot = entry
                .snapshot
                .read()
                .expect("job snapshot lock poisoned")
                .clone();
            (snapshot.id == id).then_some(snapshot)
        })
    }

    pub fn cancel(&self, user_id: u64) -> bool {
        let entry = self
            .jobs
            .lock()
            .expect("jobs mutex poisoned")
            .get(&user_id)
            .cloned();
        let Some(entry) = entry else {
            return false;
        };

        cancel_entry(&entry)
    }

    pub fn cancel_by_id(&self, id: Uuid) -> bool {
        let entry = self
            .jobs
            .lock()
            .expect("jobs mutex poisoned")
            .values()
            .find(|entry| {
                entry
                    .snapshot
                    .read()
                    .expect("job snapshot lock poisoned")
                    .id
                    == id
            })
            .cloned();
        entry.is_some_and(|entry| cancel_entry(&entry))
    }
}

fn cancel_entry(entry: &JobEntry) -> bool {
    let mut snapshot = entry.snapshot.write().expect("job snapshot lock poisoned");
    if matches!(
        snapshot.status,
        JobStatus::Completed | JobStatus::Failed | JobStatus::Cancelled
    ) {
        return false;
    }
    entry.cancellation.cancel();
    snapshot.status = JobStatus::Cancelling;
    true
}

pub struct JobLease {
    manager: Arc<JobManager>,
    entry: Arc<JobEntry>,
    permit: Option<OwnedSemaphorePermit>,
    id: Uuid,
    user_id: u64,
}

impl JobLease {
    pub fn id(&self) -> Uuid {
        self.id
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.entry.cancellation.clone()
    }

    pub fn snapshot(&self) -> JobSnapshot {
        self.entry
            .snapshot
            .read()
            .expect("job snapshot lock poisoned")
            .clone()
    }

    pub fn update(&self, update: impl FnOnce(&mut JobSnapshot)) {
        update(
            &mut self
                .entry
                .snapshot
                .write()
                .expect("job snapshot lock poisoned"),
        );
    }

    pub fn set_status(&self, status: JobStatus) {
        self.update(|snapshot| snapshot.status = status);
    }
}

impl Drop for JobLease {
    fn drop(&mut self) {
        let mut jobs = self.manager.jobs.lock().expect("jobs mutex poisoned");
        if jobs
            .get(&self.user_id)
            .is_some_and(|entry| entry.snapshot.read().expect("snapshot poisoned").id == self.id)
        {
            jobs.remove(&self.user_id);
        }
        self.permit.take();
    }
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum JobStartError {
    #[error("你已有一个运行中的任务")]
    UserBusy,
    #[error("服务当前正忙，请稍后重试")]
    GloballyBusy,
}

#[cfg(test)]
#[path = "../../tests/unit/jobs/manager.rs"]
mod tests;
