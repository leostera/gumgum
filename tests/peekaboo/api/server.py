from http.server import BaseHTTPRequestHandler, HTTPServer
import json, os

class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/healthz":
            self.send_response(200)
            self.end_headers()
            self.wfile.write(b"ok")
            return
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.end_headers()
        self.wfile.write(json.dumps({
            "service": "peekaboo-api",
            "database_bound": "DATABASE_URL" in os.environ,
            "kv_bound": "ONLINE_USERS" in os.environ,
        }).encode())

HTTPServer(("0.0.0.0", 3000), Handler).serve_forever()
