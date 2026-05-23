from __future__ import annotations

import json
import os
import time
import uuid
from pathlib import Path
from typing import Protocol

try:
    import boto3
except ImportError:  # pragma: no cover - only used outside uv-managed envs
    boto3 = None

try:
    from confluent_kafka import Producer
except ImportError:  # pragma: no cover - only used outside uv-managed envs
    Producer = None

try:
    import redis
except ImportError:  # pragma: no cover - only used outside uv-managed envs
    redis = None

from fastapi import Cookie, FastAPI, Request, Response
from fastapi.responses import PlainTextResponse

app = FastAPI(title="GumGum Visit Counter")

STATE_DIR = Path(os.environ.get("VISIT_COUNTER_STATE_DIR", "/tmp/visit-counter"))
BUCKET_DIR = Path(os.environ.get("VISIT_COUNTER_BUCKET_DIR", STATE_DIR / "bucket"))
QUEUE_DIR = Path(os.environ.get("VISIT_COUNTER_QUEUE_DIR", STATE_DIR / "queue"))
KV_PATH = Path(os.environ.get("VISIT_COUNTER_KV_PATH", STATE_DIR / "kv.json"))
KV_URL = os.environ.get("USER_COUNTERS")
KV_KEY_PREFIX = os.environ.get("USER_COUNTERS_KEY_PREFIX", "")
BUCKET_NAME = os.environ.get("VISIT_REQUESTS_BUCKET_BUCKET") or os.environ.get(
    "VISIT_REQUESTS_BUCKET", "visit-requests"
)
S3_ENDPOINT = os.environ.get("VISIT_REQUESTS_BUCKET_ENDPOINT")
S3_ACCESS_KEY_ID = os.environ.get("VISIT_REQUESTS_BUCKET_ACCESS_KEY_ID")
S3_SECRET_ACCESS_KEY = os.environ.get("VISIT_REQUESTS_BUCKET_SECRET_ACCESS_KEY")
S3_FORCE_PATH_STYLE = os.environ.get("VISIT_REQUESTS_BUCKET_FORCE_PATH_STYLE") == "true"
KAFKA_BROKERS = os.environ.get("VISIT_EVENTS_QUEUE_BROKERS")
KAFKA_TOPIC = os.environ.get("VISIT_EVENTS_QUEUE_TOPIC")


class CounterStore(Protocol):
    def increment(self, visitor_id: str) -> tuple[int, int]: ...


class RequestBucket(Protocol):
    def put_json(self, key: str, payload: dict[str, str]) -> None: ...


class EventQueue(Protocol):
    def publish(self, event_id: str, message: dict[str, str]) -> None: ...


class JsonFileCounterStore:
    def __init__(self, path: Path) -> None:
        self.path = path
        self.path.parent.mkdir(parents=True, exist_ok=True)

    def increment(self, visitor_id: str) -> tuple[int, int]:
        values = self._load()
        visitor_key = f"visitor:{visitor_id}:count"
        total_key = "visits:total"
        values[visitor_key] = int(values.get(visitor_key, 0)) + 1
        values[total_key] = int(values.get(total_key, 0)) + 1
        self.path.write_text(json.dumps(values, indent=2, sort_keys=True))
        return values[visitor_key], values[total_key]

    def _load(self) -> dict[str, int]:
        if not self.path.exists():
            return {}
        try:
            values = json.loads(self.path.read_text())
        except json.JSONDecodeError:
            return {}
        return {str(key): int(value) for key, value in values.items()}


class RedisCounterStore:
    def __init__(self, url: str, key_prefix: str = "") -> None:
        if redis is None:
            raise RuntimeError("redis is required for USER_COUNTERS-backed counters")
        self.client = redis.Redis.from_url(url, decode_responses=True)
        self.key_prefix = key_prefix

    def key(self, value: str) -> str:
        return f"{self.key_prefix}{value}"

    def increment(self, visitor_id: str) -> tuple[int, int]:
        pipe = self.client.pipeline()
        pipe.incr(self.key(f"visitor:{visitor_id}:count"))
        pipe.incr(self.key("visits:total"))
        visitor_count, total_count = pipe.execute()
        return int(visitor_count), int(total_count)


class FileRequestBucket:
    def __init__(self, root: Path) -> None:
        self.root = root
        self.root.mkdir(parents=True, exist_ok=True)

    def put_json(self, key: str, payload: dict[str, str]) -> None:
        object_path = self.root / key
        object_path.parent.mkdir(parents=True, exist_ok=True)
        object_path.write_text(json.dumps(payload, indent=2, sort_keys=True))


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

    def put_json(self, key: str, payload: dict[str, str]) -> None:
        self.client.put_object(
            Bucket=BUCKET_NAME,
            Key=key,
            Body=json.dumps(payload, indent=2, sort_keys=True).encode(),
            ContentType="application/json",
        )


class FileEventQueue:
    def __init__(self, root: Path) -> None:
        self.root = root
        self.root.mkdir(parents=True, exist_ok=True)

    def publish(self, event_id: str, message: dict[str, str]) -> None:
        (self.root / f"{event_id}.json").write_text(
            json.dumps(message, indent=2, sort_keys=True)
        )


class KafkaEventQueue:
    def __init__(self) -> None:
        if Producer is None:
            raise RuntimeError("confluent-kafka is required for Kafka-backed queue events")
        if not KAFKA_BROKERS or not KAFKA_TOPIC:
            raise RuntimeError("Kafka queue requires brokers and topic")
        self.producer = Producer({"bootstrap.servers": KAFKA_BROKERS})

    def publish(self, event_id: str, message: dict[str, str]) -> None:
        self.producer.produce(
            KAFKA_TOPIC,
            key=event_id,
            value=json.dumps(message, sort_keys=True).encode(),
        )
        self.producer.flush(timeout=5)


def s3_config():
    if not S3_FORCE_PATH_STYLE:
        return None
    from botocore.config import Config

    return Config(s3={"addressing_style": "path"})


def counter_store() -> CounterStore:
    if KV_URL and KV_URL.startswith("redis://"):
        return RedisCounterStore(KV_URL, KV_KEY_PREFIX)
    return JsonFileCounterStore(KV_PATH)


def request_bucket() -> RequestBucket:
    if S3_ENDPOINT and S3_ACCESS_KEY_ID and S3_SECRET_ACCESS_KEY:
        return S3RequestBucket()
    return FileRequestBucket(BUCKET_DIR)


def event_queue() -> EventQueue:
    if KAFKA_BROKERS and KAFKA_TOPIC:
        return KafkaEventQueue()
    return FileEventQueue(QUEUE_DIR)


def ensure_dirs() -> None:
    BUCKET_DIR.mkdir(parents=True, exist_ok=True)
    QUEUE_DIR.mkdir(parents=True, exist_ok=True)
    KV_PATH.parent.mkdir(parents=True, exist_ok=True)


def record_visit(
    path: str,
    user_agent: str,
    visitor_id: str | None,
    counters: CounterStore | None = None,
    bucket: RequestBucket | None = None,
    queue: EventQueue | None = None,
) -> tuple[str, int, int]:
    ensure_dirs()
    visitor_id = visitor_id or uuid.uuid4().hex
    counters = counters or counter_store()
    bucket = bucket or request_bucket()
    queue = queue or event_queue()
    visitor_count, total_count = counters.increment(visitor_id)
    request_id = uuid.uuid4().hex
    key = f"requests/{int(time.time())}-{request_id}.json"
    payload = {
        "id": request_id,
        "visitor_id": visitor_id,
        "path": path,
        "user_agent": user_agent,
        "seen_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "bucket_key": key,
        "visitor_count": str(visitor_count),
        "total_count": str(total_count),
    }
    bucket.put_json(key, payload)
    queue.publish(request_id, {"bucket": BUCKET_NAME, "key": key})
    return visitor_id, visitor_count, total_count


@app.get("/healthz", response_class=PlainTextResponse)
@app.get("/_/live", response_class=PlainTextResponse)
@app.get("/_/ready", response_class=PlainTextResponse)
def healthz() -> str:
    return "ok\n"


@app.get("/", response_class=PlainTextResponse)
def visit(
    request: Request, response: Response, visit_counter_id: str | None = Cookie(default=None)
) -> str:
    visitor_id, visitor_count, total_count = record_visit(
        path=request.url.path,
        user_agent=request.headers.get("User-Agent", ""),
        visitor_id=visit_counter_id,
    )
    response.set_cookie("visit_counter_id", visitor_id, path="/", samesite="lax")
    return (
        f"Hello visitor {visitor_id}, "
        f"this is your visit #{visitor_count} and site visit #{total_count}\n"
    )


def main() -> None:
    import uvicorn

    uvicorn.run(app, host="0.0.0.0", port=int(os.environ.get("PORT", "3000")))


if __name__ == "__main__":
    main()
