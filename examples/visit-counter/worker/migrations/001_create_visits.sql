CREATE TABLE IF NOT EXISTS visits (
    id TEXT PRIMARY KEY,
    visitor_id TEXT NOT NULL,
    path TEXT NOT NULL,
    user_agent TEXT NOT NULL,
    seen_at TEXT NOT NULL,
    bucket_key TEXT NOT NULL,
    processed_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
