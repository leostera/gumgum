import json
import sqlite3

from visit_counter_worker import worker


def configure_fallback_paths(monkeypatch, tmp_path):
    monkeypatch.setattr(worker, "BUCKET_DIR", tmp_path / "bucket")
    monkeypatch.setattr(worker, "QUEUE_DIR", tmp_path / "queue")
    monkeypatch.setattr(worker, "DB_PATH", tmp_path / "visits.sqlite")
    monkeypatch.setattr(worker, "DATABASE_URL", None)
    monkeypatch.setattr(worker, "MIGRATIONS_DIR", tmp_path / "missing-migrations")


def test_worker_reads_bucket_message_and_inserts_visit(monkeypatch, tmp_path):
    configure_fallback_paths(monkeypatch, tmp_path)
    request_key = "requests/1-visit.json"
    request = {
        "id": "visit-1",
        "visitor_id": "visitor-1",
        "path": "/",
        "user_agent": "pytest",
        "seen_at": "2026-05-22T00:00:00Z",
        "bucket_key": request_key,
    }
    request_path = tmp_path / "bucket" / request_key
    request_path.parent.mkdir(parents=True)
    request_path.write_text(json.dumps(request))
    message_path = tmp_path / "queue" / "visit-1.json"
    message_path.parent.mkdir(parents=True)
    message_path.write_text(json.dumps({"bucket": "visit-requests", "key": request_key}))

    store = worker.visit_store()
    try:
        worker.process_message(store, message_path)
    finally:
        store.close()

    assert not message_path.exists()
    conn = sqlite3.connect(tmp_path / "visits.sqlite")
    row = conn.execute(
        "select id, visitor_id, path, user_agent, bucket_key from visits"
    ).fetchone()
    conn.close()
    assert row == ("visit-1", "visitor-1", "/", "pytest", request_key)
