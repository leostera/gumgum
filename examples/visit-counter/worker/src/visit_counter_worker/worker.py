from __future__ import annotations

import json
import os
import sqlite3
import time
from pathlib import Path
from typing import Protocol

try:
    import boto3
except ImportError:  # pragma: no cover - only used outside uv-managed envs
    boto3 = None

try:
    from confluent_kafka import Consumer
except ImportError:  # pragma: no cover - only used outside uv-managed envs
    Consumer = None

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
BUCKET_NAME = os.environ.get("VISIT_REQUESTS_BUCKET_BUCKET") or os.environ.get(
    "VISIT_REQUESTS_BUCKET", "visit-requests"
)
S3_ENDPOINT = os.environ.get("VISIT_REQUESTS_BUCKET_ENDPOINT")
S3_ACCESS_KEY_ID = os.environ.get("VISIT_REQUESTS_BUCKET_ACCESS_KEY_ID")
S3_SECRET_ACCESS_KEY = os.environ.get("VISIT_REQUESTS_BUCKET_SECRET_ACCESS_KEY")
S3_FORCE_PATH_STYLE = os.environ.get("VISIT_REQUESTS_BUCKET_FORCE_PATH_STYLE") == "true"
KAFKA_BROKERS = os.environ.get("VISIT_EVENTS_QUEUE_BROKERS")
KAFKA_TOPIC = os.environ.get("VISIT_EVENTS_QUEUE_TOPIC")
KAFKA_GROUP_ID = os.environ.get("VISIT_EVENTS_QUEUE_GROUP_ID", "visit-counter-worker")


class VisitStore(Protocol):
    def insert_visit(self, request: dict[str, str]) -> None: ...

    def close(self) -> None: ...


class RequestBucket(Protocol):
    def get_json(self, key: str) -> dict[str, str]: ...


class EventSource(Protocol):
    def poll(self) -> list[dict[str, str]]: ...

    def acknowledge(self, message: dict[str, str]) -> None: ...

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


class FileRequestBucket:
    def __init__(self, root: Path) -> None:
        self.root = root
        self.root.mkdir(parents=True, exist_ok=True)

    def get_json(self, key: str) -> dict[str, str]:
        return json.loads((self.root / key).read_text())


class S3RequestBucket:
    def __init__(self) -> None:
        if boto3 is None:
            raise RuntimeError("boto3 is required for S3-backed request storage")
        self.client = boto3.client(
            "s3",
            endpoint_url=S3_ENDPOINT,
            aws_access_key_id=S3_ACCESS_KEY_ID,
            aws_secret_access_key=S3_SECRET_ACCESS_KEY,
            config=s3_config(),
        )

    def get_json(self, key: str) -> dict[str, str]:
        response = self.client.get_object(Bucket=BUCKET_NAME, Key=key)
        return json.loads(response["Body"].read().decode())


class FileEventSource:
    def __init__(self, root: Path) -> None:
        self.root = root
        self.root.mkdir(parents=True, exist_ok=True)

    def poll(self) -> list[dict[str, str]]:
        messages = []
        for path in sorted(self.root.glob("*.json")):
            message = json.loads(path.read_text())
            message["_path"] = str(path)
            messages.append(message)
        return messages

    def acknowledge(self, message: dict[str, str]) -> None:
        if path := message.get("_path"):
            Path(path).unlink(missing_ok=True)

    def close(self) -> None:
        pass


class KafkaEventSource:
    def __init__(self) -> None:
        if Consumer is None:
            raise RuntimeError("confluent-kafka is required for Kafka-backed queue events")
        if not KAFKA_BROKERS or not KAFKA_TOPIC:
            raise RuntimeError("Kafka queue requires brokers and topic")
        self.consumer = Consumer(
            {
                "bootstrap.servers": KAFKA_BROKERS,
                "group.id": KAFKA_GROUP_ID,
                "auto.offset.reset": "earliest",
                "enable.auto.commit": False,
            }
        )
        self.consumer.subscribe([KAFKA_TOPIC])

    def poll(self) -> list[dict[str, str]]:
        message = self.consumer.poll(timeout=POLL_SECONDS)
        if message is None:
            return []
        if message.error():
            raise RuntimeError(message.error())
        payload = json.loads(message.value().decode())
        payload["_kafka_message"] = message
        return [payload]

    def acknowledge(self, message: dict[str, str]) -> None:
        kafka_message = message.get("_kafka_message")
        if kafka_message is not None:
            self.consumer.commit(kafka_message)

    def close(self) -> None:
        self.consumer.close()


def s3_config():
    if not S3_FORCE_PATH_STYLE:
        return None
    from botocore.config import Config

    return Config(s3={"addressing_style": "path"})


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


def request_bucket() -> RequestBucket:
    if S3_ENDPOINT and S3_ACCESS_KEY_ID and S3_SECRET_ACCESS_KEY:
        return S3RequestBucket()
    return FileRequestBucket(BUCKET_DIR)


def event_source() -> EventSource:
    if KAFKA_BROKERS and KAFKA_TOPIC:
        return KafkaEventSource()
    return FileEventSource(QUEUE_DIR)


def process_message(
    store: VisitStore,
    message: dict[str, str],
    bucket: RequestBucket | None = None,
    events: EventSource | None = None,
) -> None:
    bucket = bucket or request_bucket()
    request = bucket.get_json(message["key"])
    store.insert_visit(request)
    if events is not None:
        events.acknowledge(message)
    print(f"processed visit {request['id']} from {request['bucket_key']}", flush=True)


def main() -> None:
    BUCKET_DIR.mkdir(parents=True, exist_ok=True)
    QUEUE_DIR.mkdir(parents=True, exist_ok=True)
    store = visit_store()
    bucket = request_bucket()
    events = event_source()
    print("visit-counter worker ready", flush=True)
    try:
        while True:
            for message in events.poll():
                try:
                    process_message(store, message, bucket=bucket, events=events)
                except Exception as error:  # pragma: no cover - runtime diagnostics
                    print(f"failed to process {message}: {error}", flush=True)
            time.sleep(POLL_SECONDS)
    finally:
        events.close()
        store.close()


if __name__ == "__main__":
    main()
