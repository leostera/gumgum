from http.server import BaseHTTPRequestHandler, HTTPServer
import json
import os

PORT = int(os.environ.get("PORT", "3000"))


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/healthz":
            self.send_response(200)
            self.send_header("content-type", "application/json")
            self.end_headers()
            self.wfile.write(b'{"status":"ok"}')
            return

        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.end_headers()
        body = {"message": "gum guuuum...hellooo! 👊"}
        self.wfile.write(json.dumps(body).encode())

    def log_message(self, format, *args):
        print("%s - %s" % (self.address_string(), format % args), flush=True)


HTTPServer(("0.0.0.0", PORT), Handler).serve_forever()
