from __future__ import annotations

import json
import os
import sqlite3
import time
from pathlib import Path

STATE_DIR = Path(os.environ.get("VISIT_COUNTER_STATE_DIR", "/tmp/visit-counter"))
BUCKET_DIR = Path(os.environ.get("VISIT_COUNTER_BUCKET_DIR", STATE_DIR / "bucket"))
QUEUE_DIR = Path(os.environ.get("VISIT_COUNTER_QUEUE_DIR", STATE_DIR / "queue"))
DB_PATH = Path(os.environ.get("VISIT_COUNTER_DB_PATH", STATE_DIR / "visits.sqlite"))
POLL_SECONDS = float(os.environ.get("VISIT_COUNTER_POLL_SECONDS", "1"))


def ensure_db() -> sqlite3.Connection:
    DB_PATH.parent.mkdir(parents=True, exist_ok=True)
    conn = sqlite3.connect(DB_PATH)
    conn.execute(
        """
        CREATE TABLE IF NOT EXISTS visits (
            id TEXT PRIMARY KEY,
            visitor_id TEXT NOT NULL,
            path TEXT NOT NULL,
            user_agent TEXT NOT NULL,
            seen_at TEXT NOT NULL,
            bucket_key TEXT NOT NULL,
            processed_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        )
        """
    )
    conn.commit()
    return conn


def process_message(conn: sqlite3.Connection, message_path: Path) -> None:
    message = json.loads(message_path.read_text())
    request_path = BUCKET_DIR / message["key"]
    request = json.loads(request_path.read_text())
    conn.execute(
        """
        INSERT OR REPLACE INTO visits (id, visitor_id, path, user_agent, seen_at, bucket_key)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        """,
        (
            request["id"],
            request["visitor_id"],
            request["path"],
            request["user_agent"],
            request["seen_at"],
            request["bucket_key"],
        ),
    )
    conn.commit()
    message_path.unlink()
    print(f"processed visit {request['id']} from {request['bucket_key']}", flush=True)


def main() -> None:
    BUCKET_DIR.mkdir(parents=True, exist_ok=True)
    QUEUE_DIR.mkdir(parents=True, exist_ok=True)
    conn = ensure_db()
    print("visit-counter worker ready", flush=True)
    while True:
        for message_path in sorted(QUEUE_DIR.glob("*.json")):
            try:
                process_message(conn, message_path)
            except Exception as error:  # pragma: no cover - runtime diagnostics
                print(f"failed to process {message_path}: {error}", flush=True)
        time.sleep(POLL_SECONDS)


if __name__ == "__main__":
    main()
