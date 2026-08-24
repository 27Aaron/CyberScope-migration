CREATE TABLE searches (
    id TEXT PRIMARY KEY NOT NULL,
    query TEXT NOT NULL,
    fields_json TEXT NOT NULL,
    format TEXT NOT NULL,
    max_results INTEGER NOT NULL,
    full INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL,
    matched_size INTEGER,
    written_rows INTEGER NOT NULL DEFAULT 0,
    upstream_attempts INTEGER NOT NULL DEFAULT 0,
    retries INTEGER NOT NULL DEFAULT 0,
    possible_duplicate_charge INTEGER NOT NULL DEFAULT 0,
    error_code TEXT,
    error_message TEXT,
    created_at TEXT NOT NULL,
    started_at TEXT,
    completed_at TEXT,
    updated_at TEXT NOT NULL
);

CREATE TABLE search_results (
    search_id TEXT NOT NULL REFERENCES searches(id) ON DELETE CASCADE,
    row_number INTEGER NOT NULL,
    row_json TEXT NOT NULL,
    PRIMARY KEY (search_id, row_number)
);

CREATE INDEX search_results_search_id_idx ON search_results(search_id);
