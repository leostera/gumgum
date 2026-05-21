from __future__ import annotations

import json
import os
import time
import uuid
from pathlib import Path

from fastapi import Cookie, FastAPI, Response
from fastapi.responses import PlainTextResponse

app = FastAPI(title="GumGum Visit Counter")

STATE_DIR = Path(os.environ.get("VISIT_COUNTER_STATE_DIR", "/tmp/visit-counter"))
BUCKET_DIR = Path(os.environ.get("VISIT_COUNTER_BUCKET_DIR", STATE_DIR / "bucket"))
QUEUE_DIR = Path(os.environ.get("VISIT_COUNTER_QUEUE_DIR", STATE_DIR / "queue"))
KV_PATH = Path(os.environ.get("VISIT_COUNTER_KV_PATH", STATE_DIR / "kv.json"))


def ensure_dirs() -> None:
    BUCKET_DIR.mkdir(parents=True, exist_ok=True)
    QUEUE_DIR.mkdir(parents=True, exist_ok=True)
    KV_PATH.parent.mkdir(parents=True, exist_ok=True)


def load_kv() -> dict[str, int]:
    if not KV_PATH.exists():
        return {}
    try:
        values = json.loads(KV_PATH.read_text())
    except json.JSONDecodeError:
        return {}
    return {str(key): int(value) for key, value in values.items()}


def save_kv(values: dict[str, int]) -> None:
    KV_PATH.write_text(json.dumps(values, indent=2, sort_keys=True))


def increment_counter(visitor_id: str) -> int:
    values = load_kv()
    key = f"visitor:{visitor_id}:count"
    values[key] = int(values.get(key, 0)) + 1
    save_kv(values)
    return values[key]


@app.get("/healthz", response_class=PlainTextResponse)
def healthz() -> str:
    return "ok\n"


@app.get("/", response_class=PlainTextResponse)
def visit(response: Response, visit_counter_id: str | None = Cookie(default=None)) -> str:
    ensure_dirs()
    visitor_id = visit_counter_id or uuid.uuid4().hex
    count = increment_counter(visitor_id)
    request_id = uuid.uuid4().hex
    key = f"requests/{int(time.time())}-{request_id}.json"
    payload = {
        "id": request_id,
        "visitor_id": visitor_id,
        "path": "/",
        "user_agent": "",  # The real provider-backed version will capture headers/traces.
        "seen_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "bucket_key": key,
    }

    object_path = BUCKET_DIR / key
    object_path.parent.mkdir(parents=True, exist_ok=True)
    object_path.write_text(json.dumps(payload, indent=2, sort_keys=True))

    queue_message = {
        "bucket": os.environ.get("VISIT_REQUESTS_BUCKET", "visit-requests"),
        "key": key,
    }
    (QUEUE_DIR / f"{request_id}.json").write_text(
        json.dumps(queue_message, indent=2, sort_keys=True)
    )

    response.set_cookie("visit_counter_id", visitor_id, path="/", samesite="lax")
    return f"Hello visitor {visitor_id}, visit #{count}\n"


def main() -> None:
    import uvicorn

    uvicorn.run(app, host="0.0.0.0", port=int(os.environ.get("PORT", "3000")))


if __name__ == "__main__":
    main()
