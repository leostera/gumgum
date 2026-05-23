import json

from visit_counter_api import server


def configure_fallback_paths(monkeypatch, tmp_path):
    monkeypatch.setattr(server, "STATE_DIR", tmp_path)
    monkeypatch.setattr(server, "BUCKET_DIR", tmp_path / "bucket")
    monkeypatch.setattr(server, "QUEUE_DIR", tmp_path / "queue")
    monkeypatch.setattr(server, "KV_PATH", tmp_path / "kv.json")
    monkeypatch.setattr(server, "KV_URL", None)
    monkeypatch.setattr(server, "KV_KEY_PREFIX", "")
    monkeypatch.setattr(server, "S3_ENDPOINT", None)
    monkeypatch.setattr(server, "S3_ACCESS_KEY_ID", None)
    monkeypatch.setattr(server, "S3_SECRET_ACCESS_KEY", None)
    monkeypatch.setattr(server, "KAFKA_BROKERS", None)
    monkeypatch.setattr(server, "KAFKA_TOPIC", None)


def test_visit_increments_counter_and_writes_bucket_and_queue(monkeypatch, tmp_path):
    configure_fallback_paths(monkeypatch, tmp_path)

    first_visitor, first_count, first_total = server.record_visit(
        path="/", user_agent="pytest", visitor_id="visitor-1"
    )
    second_visitor, second_count, second_total = server.record_visit(
        path="/", user_agent="pytest", visitor_id="visitor-1"
    )
    third_visitor, third_count, third_total = server.record_visit(
        path="/", user_agent="pytest", visitor_id="visitor-2"
    )

    assert (first_visitor, first_count, first_total) == ("visitor-1", 1, 1)
    assert (second_visitor, second_count, second_total) == ("visitor-1", 2, 2)
    assert (third_visitor, third_count, third_total) == ("visitor-2", 1, 3)
    kv = json.loads((tmp_path / "kv.json").read_text())
    assert kv["visitor:visitor-1:count"] == 2
    assert kv["visitor:visitor-2:count"] == 1
    assert kv["visits:total"] == 3

    request_objects = sorted((tmp_path / "bucket" / "requests").glob("*.json"))
    queue_messages = sorted((tmp_path / "queue").glob("*.json"))
    assert len(request_objects) == 3
    assert len(queue_messages) == 3

    requests = [json.loads(path.read_text()) for path in request_objects]
    messages = [json.loads(path.read_text()) for path in queue_messages]
    assert {request["visitor_id"] for request in requests} == {"visitor-1", "visitor-2"}
    assert all(request["user_agent"] == "pytest" for request in requests)
    assert all(request["bucket_key"].startswith("requests/") for request in requests)
    assert {request["visitor_count"] for request in requests} == {"1", "2"}
    assert {request["total_count"] for request in requests} == {"1", "2", "3"}
    assert all(message["bucket"] == "visit-requests" for message in messages)
    assert all(message["key"].startswith("requests/") for message in messages)
