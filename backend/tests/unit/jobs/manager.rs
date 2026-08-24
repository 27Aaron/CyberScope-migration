use super::*;

#[test]
fn enforces_global_slot_and_cleans_up_on_drop() {
    let manager = JobManager::new(1);
    let first = manager.try_start(1, JobKind::Search).unwrap();
    assert_eq!(
        manager.try_start(2, JobKind::Search).err().unwrap(),
        JobStartError::GloballyBusy
    );

    drop(first);
    assert!(manager.snapshot(1).is_none());
    assert!(manager.try_start(2, JobKind::Search).is_ok());
}

#[test]
fn prevents_duplicate_user_task() {
    let manager = JobManager::new(2);
    let _first = manager.try_start(1, JobKind::Search).unwrap();
    assert_eq!(
        manager.try_start(1, JobKind::Batch).err().unwrap(),
        JobStartError::UserBusy
    );
}

#[test]
fn cancellation_updates_status_and_token() {
    let manager = JobManager::new(1);
    let job = manager.try_start(1, JobKind::Search).unwrap();
    let token = job.cancellation_token();
    assert!(manager.cancel(1));
    assert!(token.is_cancelled());
    assert_eq!(manager.snapshot(1).unwrap().status, JobStatus::Cancelling);
}
