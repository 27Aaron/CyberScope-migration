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

#[tokio::test]
async fn connect_fails_stale_queued_and_running_records_from_a_previous_process() {
    let directory = tempfile::tempdir().unwrap();

    // Seed in-flight records left by a crashed process.
    {
        let store = SearchStore::connect(directory.path()).await.unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        for status in ["queued", "running", "cancelling"] {
            sqlx::query(
                "INSERT INTO searches (id, query, fields_json, format, max_results, full, status, \
                 matched_size, written_rows, upstream_attempts, retries, possible_duplicate_charge, \
                 error_code, error_message, created_at, updated_at) \
                 VALUES (?, 'domain=\"a.com\"', '[]', 'csv', 100, 0, ?, 0, 0, 0, 0, 0, NULL, NULL, ?, ?)",
            )
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(status)
            .bind(&now)
            .bind(&now)
            .execute(store.pool())
            .await
            .unwrap();
        }
    }

    // Reconnecting should mark all in-flight records as stale failures.
    let store = SearchStore::connect(directory.path()).await.unwrap();
    let rows = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        "SELECT status, error_code, error_message FROM searches ORDER BY status",
    )
    .fetch_all(store.pool())
    .await
    .unwrap();

    assert_eq!(rows.len(), 3);
    for (status, error_code, error_message) in rows {
        assert_eq!(status, "failed");
        assert_eq!(error_code.as_deref(), Some("stale_run"));
        assert!(error_message.unwrap().contains("重启"));
    }
}

#[tokio::test]
async fn connect_keeps_terminal_records_untouched() {
    let directory = tempfile::tempdir().unwrap();

    {
        let store = SearchStore::connect(directory.path()).await.unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        for status in ["completed", "failed", "cancelled"] {
            sqlx::query(
                "INSERT INTO searches (id, query, fields_json, format, max_results, full, status, \
                 matched_size, written_rows, upstream_attempts, retries, possible_duplicate_charge, \
                 error_code, error_message, created_at, updated_at) \
                 VALUES (?, 'domain=\"a.com\"', '[]', 'csv', 100, 0, ?, 0, 0, 0, 0, 0, NULL, NULL, ?, ?)",
            )
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(status)
            .bind(&now)
            .bind(&now)
            .execute(store.pool())
            .await
            .unwrap();
        }
    }

    let store = SearchStore::connect(directory.path()).await.unwrap();
    let statuses = sqlx::query_scalar::<_, String>("SELECT status FROM searches ORDER BY status")
        .fetch_all(store.pool())
        .await
        .unwrap();

    assert_eq!(statuses, vec!["cancelled", "completed", "failed"]);
}
