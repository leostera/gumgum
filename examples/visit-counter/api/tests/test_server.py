import json

from fastapi import Response

from visit_counter_api import server


def configure_fallback_paths(monkeypatch, tmp_path):
    monkeypatch.setattr(server, "STATE_DIR", tmp_path)
    monkeypatch.setattr(server, "BUCKET_DIR", tmp_path / "bucket")
    monkeypatch.setattr(server, "QUEUE_DIR", tmp_path / "queue")
    monkeypatch.setattr(server, "KV_PATH", tmp_path / "kv.json")


def test_visit_increments_counter_and_writes_bucket_and_queue(monkeypatch, tmp_path):
    configure_fallback_paths(monkeypatch, tmp_path)

    first = server.visit(Response(), visit_counter_id="visitor-1")
    second = server.visit(Response(), visit_counter_id="visitor-1")

    assert first == "Hello visitor visitor-1, visit #1\n"
    assert second == "Hello visitor visitor-1, visit #2\n"
    kv = json.loads((tmp_path / "kv.json").read_text())
    assert kv["visitor:visitor-1:count"] == 2

    request_objects = sorted((tmp_path / "bucket" / "requests").glob("*.json"))
    queue_messages = sorted((tmp_path / "queue").glob("*.json"))
    assert len(request_objects) == 2
    assert len(queue_messages) == 2

    request = json.loads(request_objects[0].read_text())
    message = json.loads(queue_messages[0].read_text())
    assert request["visitor_id"] == "visitor-1"
    assert request["bucket_key"].startswith("requests/")
    assert message["bucket"] == "visit-requests"
    assert message["key"].startswith("requests/")
