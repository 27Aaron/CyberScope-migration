use super::*;

#[test]
fn search_status_round_trips_to_storage_value() {
    for (status, value) in [
        (SearchStatus::Queued, "queued"),
        (SearchStatus::Running, "running"),
        (SearchStatus::Cancelling, "cancelling"),
        (SearchStatus::Completed, "completed"),
        (SearchStatus::Failed, "failed"),
        (SearchStatus::Cancelled, "cancelled"),
    ] {
        assert_eq!(status.as_str(), value);
        assert_eq!(SearchStatus::try_from(value), Ok(status));
    }
}

#[test]
fn unknown_search_status_is_rejected() {
    assert_eq!(SearchStatus::try_from("unknown"), Err(()));
}

#[tokio::test]
async fn connect_creates_database_file_under_configured_directory() {
    let directory = tempfile::tempdir().unwrap();

    let store = SearchStore::connect(directory.path()).await.unwrap();

    assert!(directory.path().join("cyberscope.db").is_file());
    drop(store);
}
