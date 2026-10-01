"""P7b / QG5: logging dead-end HTTP proxy (Python stdlib only).

Started before the app runs with HTTP_PROXY/HTTPS_PROXY/ALL_PROXY pointing here and
NO_PROXY=localhost,127.0.0.1,::1. Every proxied request (CONNECT host:port for HTTPS,
absolute URL for HTTP) is written as one JSON line to --log and answered with 403, so
nothing leaves the machine through a proxy-aware client (reqwest, ureq, hf-hub).
Catches connections that live shorter than the socket poller's interval.

    python p7b_proxy_log.py --port 18089 --log proxy.jsonl --stop-file stop.flag
"""
import argparse
import json
import os
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


class Handler(BaseHTTPRequestHandler):
    log_path = ""
    lock = threading.Lock()

    def _record(self):
        entry = {
            "ts": time.strftime("%Y-%m-%dT%H:%M:%S"),
            "method": self.command,
            "target": self.path,
            "user_agent": self.headers.get("User-Agent", ""),
            "client": "%s:%s" % self.client_address[:2],
        }
        with Handler.lock, open(Handler.log_path, "a", encoding="utf-8") as fh:
            fh.write(json.dumps(entry) + "\n")
        self.send_response(403, "blocked by p7b offline proxy")
        self.send_header("Content-Length", "0")
        self.send_header("Connection", "close")
        self.end_headers()

    do_CONNECT = do_GET = do_POST = do_PUT = do_HEAD = do_DELETE = do_PATCH = do_OPTIONS = _record

    def log_message(self, fmt, *args):  # keep stderr quiet
        pass


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=18089)
    ap.add_argument("--log", required=True)
    ap.add_argument("--stop-file", required=True)
    args = ap.parse_args()
    Handler.log_path = args.log
    open(args.log, "a", encoding="utf-8").close()
    server = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    while not os.path.exists(args.stop_file):
        time.sleep(0.5)
    server.shutdown()


if __name__ == "__main__":
    main()
