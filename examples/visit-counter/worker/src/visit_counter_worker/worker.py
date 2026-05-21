from __future__ import annotations

import json
import os
import sqlite3
import time
from pathlib import Path
from typing import Protocol

try:  # Optional until GumGum projects DATABASE_URL from a real provider.
    import psycopg
except ImportError:  # pragma: no cover - only used outside uv-managed envs
    psycopg = None

STATE_DIR = Path(os.environ.get("VISIT_COUNTER_STATE_DIR", "/tmp/visit-counter"))
BUCKET_DIR = Path(os.environ.get("VISIT_COUNTER_BUCKET_DIR", STATE_DIR / "bucket"))
QUEUE_DIR = Path(os.environ.get("VISIT_COUNTER_QUEUE_DIR", STATE_DIR / "queue"))
DB_PATH = Path(os.environ.get("VISIT_COUNTER_DB_PATH", STATE_DIR / "visits.sqlite"))
DATABASE_URL = os.environ.get("DATABASE_URL")
POLL_SECONDS = float(os.environ.get("VISIT_COUNTER_POLL_SECONDS", "1"))
MIGRATIONS_DIR = Path(os.environ.get("VISIT_COUNTER_MIGRATIONS_DIR", "/app/migrations"))
LOCAL_MIGRATIONS_DIR = Path(__file__).resolve().parents[2] / "migrations"


class VisitStore(Protocol):
    def insert_visit(self, request: dict[str, str]) -> None: ...

    def close(self) -> None: ...


class SqliteVisitStore:
    def __init__(self, path: Path) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        self.conn = sqlite3.connect(path)
        self.apply_migrations()

    def apply_migrations(self) -> None:
        for migration in migration_files():
            self.conn.executescript(migration.read_text())
        self.conn.commit()

    def insert_visit(self, request: dict[str, str]) -> None:
        self.conn.execute(
            """
            INSERT OR REPLACE INTO visits (id, visitor_id, path, user_agent, seen_at, bucket_key)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            """,
            visit_values(request),
        )
        self.conn.commit()

    def close(self) -> None:
        self.conn.close()


class PostgresVisitStore:
    def __init__(self, database_url: str) -> None:
        if psycopg is None:
            raise RuntimeError("psycopg is required for DATABASE_URL-backed visits")
        self.conn = psycopg.connect(database_url)
        self.apply_migrations()

    def apply_migrations(self) -> None:
        with self.conn.cursor() as cursor:
            for migration in migration_files():
                cursor.execute(migration.read_text())
        self.conn.commit()

    def insert_visit(self, request: dict[str, str]) -> None:
        with self.conn.cursor() as cursor:
            cursor.execute(
                """
                INSERT INTO visits (id, visitor_id, path, user_agent, seen_at, bucket_key)
                VALUES (%s, %s, %s, %s, %s, %s)
                ON CONFLICT (id) DO UPDATE SET
                    visitor_id = EXCLUDED.visitor_id,
                    path = EXCLUDED.path,
                    user_agent = EXCLUDED.user_agent,
                    seen_at = EXCLUDED.seen_at,
                    bucket_key = EXCLUDED.bucket_key
                """,
                visit_values(request),
            )
        self.conn.commit()

    def close(self) -> None:
        self.conn.close()


def migration_files() -> list[Path]:
    directory = MIGRATIONS_DIR if MIGRATIONS_DIR.exists() else LOCAL_MIGRATIONS_DIR
    return sorted(directory.glob("*.sql"))


def visit_values(request: dict[str, str]) -> tuple[str, str, str, str, str, str]:
    return (
        request["id"],
        request["visitor_id"],
        request["path"],
        request["user_agent"],
        request["seen_at"],
        request["bucket_key"],
    )


def visit_store() -> VisitStore:
    if DATABASE_URL and DATABASE_URL.startswith(("postgres://", "postgresql://")):
        print("using Postgres DATABASE_URL visit store", flush=True)
        return PostgresVisitStore(DATABASE_URL)
    print(f"using SQLite fallback visit store at {DB_PATH}", flush=True)
    return SqliteVisitStore(DB_PATH)


def process_message(store: VisitStore, message_path: Path) -> None:
    message = json.loads(message_path.read_text())
    request_path = BUCKET_DIR / message["key"]
    request = json.loads(request_path.read_text())
    store.insert_visit(request)
    message_path.unlink()
    print(f"processed visit {request['id']} from {request['bucket_key']}", flush=True)


def main() -> None:
    BUCKET_DIR.mkdir(parents=True, exist_ok=True)
    QUEUE_DIR.mkdir(parents=True, exist_ok=True)
    store = visit_store()
    print("visit-counter worker ready", flush=True)
    try:
        while True:
            for message_path in sorted(QUEUE_DIR.glob("*.json")):
                try:
                    process_message(store, message_path)
                except Exception as error:  # pragma: no cover - runtime diagnostics
                    print(f"failed to process {message_path}: {error}", flush=True)
            time.sleep(POLL_SECONDS)
    finally:
        store.close()


if __name__ == "__main__":
    main()
