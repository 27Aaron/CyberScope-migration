mod manager;
mod runner;

pub use manager::{JobKind, JobLease, JobManager, JobSnapshot, JobStartError, JobStatus};
pub use runner::{
    BatchCompletion, BatchFailure, BatchOutcome, RunError, RunSettings, SingleOutcome, run_batch,
    run_single,
};
