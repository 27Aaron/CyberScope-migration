use std::path::Path;

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{
    FromRow, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use uuid::Uuid;

pub const SINGLE_USER_ID: u64 = 0;
pub const DEFAULT_MAX_RESULTS: u64 = 100;
pub const MAX_RESULTS: u64 = 10_000;

#[derive(Clone)]
pub struct SearchStore {
    pool: SqlitePool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchStatus {
    Queued,
    Running,
    Cancelling,
    Completed,
    Failed,
    Cancelled,
}

impl SearchStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Cancelling => "cancelling",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

impl TryFrom<&str> for SearchStatus {
    type Error = ();

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "cancelling" => Ok(Self::Cancelling),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SearchRecord {
    pub id: Uuid,
    pub query: String,
    pub fields: Vec<String>,
    pub format: String,
    pub max_results: u64,
    pub full: bool,
    pub status: SearchStatus,
    pub matched_size: Option<u64>,
    pub written_rows: u64,
    pub upstream_attempts: u64,
    pub retries: u64,
    pub possible_duplicate_charge: bool,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(FromRow)]
struct SearchRow {
    id: String,
    query: String,
    fields_json: String,
    format: String,
    max_results: i64,
    full: i64,
    status: String,
    matched_size: Option<i64>,
    written_rows: i64,
    upstream_attempts: i64,
    retries: i64,
    possible_duplicate_charge: i64,
    error_code: Option<String>,
    error_message: Option<String>,
    created_at: String,
    started_at: Option<String>,
    completed_at: Option<String>,
    updated_at: String,
}

impl SearchStore {
    pub async fn connect(database_path: &Path) -> anyhow::Result<Self> {
        tokio::fs::create_dir_all(database_path).await?;
        let database_file = database_path.join("cyberscope.db");
        let options = SqliteConnectOptions::new()
            .filename(database_file)
            .create_if_missing(true)
            // Apply foreign-key enforcement to every pooled connection.
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        let store = Self { pool };
        store.fail_stale_records().await?;
        Ok(store)
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Mark in-flight searches as failed after a process restart.
    ///
    /// This prevents interrupted jobs from remaining permanently active.
    async fn fail_stale_records(&self) -> Result<(), sqlx::Error> {
        let now = Utc::now().to_rfc3339();
        let result = sqlx::query(
            "UPDATE searches SET status = 'failed', \
             error_code = 'stale_run', error_message = '进程重启，任务未能完成', \
             completed_at = ?, updated_at = ? \
             WHERE status IN ('queued', 'running', 'cancelling')",
        )
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() > 0 {
            tracing::warn!(
                count = result.rows_affected(),
                event = "stale_searches_failed",
                "启动时将残留的 queued/running 任务标记为失败"
            );
        }
        Ok(())
    }

    pub async fn insert(&self, record: &SearchRecord) -> Result<(), sqlx::Error> {
        let fields_json =
            serde_json::to_string(&record.fields).expect("field names are serializable");
        sqlx::query(
            "INSERT INTO searches
             (id, query, fields_json, format, max_results, full, status, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(record.id.to_string())
        .bind(&record.query)
        .bind(fields_json)
        .bind(&record.format)
        .bind(record.max_results as i64)
        .bind(i64::from(record.full))
        .bind(record.status.as_str())
        .bind(record.created_at.to_rfc3339())
        .bind(record.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get(&self, id: Uuid) -> Result<Option<SearchRecord>, sqlx::Error> {
        let row = sqlx::query_as::<_, SearchRow>("SELECT * FROM searches WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.map(SearchRow::try_into)
            .transpose()
            .map_err(|_| sqlx::Error::Protocol("invalid search record".into()))
    }

    pub async fn mark_running(&self, id: Uuid) -> Result<(), sqlx::Error> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "UPDATE searches SET status = 'running', started_at = COALESCE(started_at, ?), updated_at = ? WHERE id = ?",
        )
        .bind(&now)
        .bind(&now)
        .bind(id.to_string())
        .execute(&self.pool)
        .await
        .map(|_| ())
    }

    pub async fn mark_cancelling(&self, id: Uuid) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            "UPDATE searches SET status = 'cancelling', updated_at = ?
             WHERE id = ? AND status IN ('queued', 'running')",
        )
        .bind(Utc::now().to_rfc3339())
        .bind(id.to_string())
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn finish_success(
        &self,
        id: Uuid,
        matched_size: Option<u64>,
        rows: &[Vec<Value>],
        upstream_attempts: u64,
        retries: u64,
        possible_duplicate_charge: bool,
    ) -> Result<(), sqlx::Error> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("DELETE FROM search_results WHERE search_id = ?")
            .bind(id.to_string())
            .execute(&mut *transaction)
            .await?;
        for (index, row) in rows.iter().enumerate() {
            sqlx::query(
                "INSERT INTO search_results (search_id, row_number, row_json) VALUES (?, ?, ?)",
            )
            .bind(id.to_string())
            .bind(index as i64)
            .bind(
                serde_json::to_string(row)
                    .map_err(|error| sqlx::Error::Protocol(error.to_string()))?,
            )
            .execute(&mut *transaction)
            .await?;
        }
        sqlx::query(
            "UPDATE searches SET status = 'completed', matched_size = ?, written_rows = ?,
             upstream_attempts = ?, retries = ?, possible_duplicate_charge = ?, completed_at = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(matched_size.map(|value| value as i64))
        .bind(rows.len() as i64)
        .bind(upstream_attempts as i64)
        .bind(retries as i64)
        .bind(i64::from(possible_duplicate_charge))
        .bind(Utc::now().to_rfc3339())
        .bind(Utc::now().to_rfc3339())
        .bind(id.to_string())
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await
    }

    pub async fn finish_cancelled(&self, id: Uuid) -> Result<(), sqlx::Error> {
        self.update_status(id, SearchStatus::Cancelled, None, None)
            .await
    }

    pub async fn finish_failed(
        &self,
        id: Uuid,
        code: &str,
        message: &str,
    ) -> Result<(), sqlx::Error> {
        self.update_status(id, SearchStatus::Failed, Some((code, message)), None)
            .await
    }

    pub async fn results(&self, id: Uuid) -> Result<Vec<Vec<Value>>, sqlx::Error> {
        let rows = sqlx::query_as::<_, (String,)>(
            "SELECT row_json FROM search_results WHERE search_id = ? ORDER BY row_number",
        )
        .bind(id.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|(row,)| {
                serde_json::from_str(&row).map_err(|error| sqlx::Error::Protocol(error.to_string()))
            })
            .collect()
    }

    async fn update_status(
        &self,
        id: Uuid,
        status: SearchStatus,
        error: Option<(&str, &str)>,
        completed_at: Option<DateTime<Utc>>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE searches SET status = ?, error_code = ?, error_message = ?, completed_at = ?, updated_at = ? WHERE id = ?",
        )
        .bind(status.as_str())
        .bind(error.map(|(code, _)| code))
        .bind(error.map(|(_, message)| message))
        .bind(completed_at.map(|value| value.to_rfc3339()).or_else(|| {
            (status == SearchStatus::Failed || status == SearchStatus::Cancelled).then(|| Utc::now().to_rfc3339())
        }))
        .bind(Utc::now().to_rfc3339())
        .bind(id.to_string())
        .execute(&self.pool)
        .await
        .map(|_| ())
    }
}

impl TryFrom<SearchRow> for SearchRecord {
    type Error = ();

    fn try_from(row: SearchRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: Uuid::parse_str(&row.id).map_err(|_| ())?,
            query: row.query,
            fields: serde_json::from_str(&row.fields_json).map_err(|_| ())?,
            format: row.format,
            max_results: u64::try_from(row.max_results).map_err(|_| ())?,
            full: row.full != 0,
            status: SearchStatus::try_from(row.status.as_str())?,
            matched_size: row
                .matched_size
                .map(u64::try_from)
                .transpose()
                .map_err(|_| ())?,
            written_rows: u64::try_from(row.written_rows).map_err(|_| ())?,
            upstream_attempts: u64::try_from(row.upstream_attempts).map_err(|_| ())?,
            retries: u64::try_from(row.retries).map_err(|_| ())?,
            possible_duplicate_charge: row.possible_duplicate_charge != 0,
            error_code: row.error_code,
            error_message: row.error_message,
            created_at: DateTime::parse_from_rfc3339(&row.created_at)
                .map_err(|_| ())?
                .with_timezone(&Utc),
            started_at: row
                .started_at
                .map(|value| {
                    DateTime::parse_from_rfc3339(&value).map(|date| date.with_timezone(&Utc))
                })
                .transpose()
                .map_err(|_| ())?,
            completed_at: row
                .completed_at
                .map(|value| {
                    DateTime::parse_from_rfc3339(&value).map(|date| date.with_timezone(&Utc))
                })
                .transpose()
                .map_err(|_| ())?,
            updated_at: DateTime::parse_from_rfc3339(&row.updated_at)
                .map_err(|_| ())?
                .with_timezone(&Utc),
        })
    }
}

#[cfg(test)]
#[path = "../tests/unit/searches.rs"]
mod tests;
