#!/usr/bin/env python3
"""Test an already-built CLI against a temporary, untrusted HTTPS endpoint.

Usage: python3 tools/test-tls-policy.py --cli target/debug/dvs
Requires Python 3 and openssl. No trust-store changes or real credentials are used.
"""

import argparse
import http.server
import json
import os
from pathlib import Path
import ssl
import subprocess
import tempfile
import threading

WARNING = "TLS certificate verification is disabled via DVS_INSECURE"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", required=True, type=Path)
    cli = parser.parse_args().cli.resolve()
    with tempfile.TemporaryDirectory(prefix="dvs-tls-policy-") as directory:
        root = Path(directory)
        subprocess.run(
            ["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes",
             "-days", "1", "-keyout", str(root / "key.pem"), "-out",
             str(root / "cert.pem"), "-subj", "/CN=localhost", "-addext",
             "subjectAltName=DNS:localhost"],
            check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
        hits = []

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_POST(self):
                hits.append(self.path)
                if self.path.endswith("/authorize"):
                    body = b'{"device_code":"tls-test","user_code":"tls-test"}'
                    self.send_response(200)
                else:
                    body = b"{}"
                    self.send_response(400)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *args):
                pass

        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(root / "cert.pem", root / "key.pem")
        server.socket = context.wrap_socket(server.socket, server_side=True)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        results = []
        try:
            for value in [None, "", "0", "false", "yes", "on", " true", "true ",
                          "1", "true", "TRUE", "TrUe"]:
                env = os.environ.copy()
                env.pop("DVS_INSECURE", None)
                env["RUST_LOG"] = "warn"
                env["XDG_CONFIG_HOME"] = str(root / "config")
                if value is not None:
                    env["DVS_INSECURE"] = value
                before = len(hits)
                run = subprocess.run(
                    [str(cli), "attach", "tls-test",
                     f"https://localhost:{server.server_port}/"],
                    cwd=root, env=env, text=True, capture_output=True, timeout=30,
                )
                count = len(hits) - before
                warnings = run.stderr.count(WARNING)
                insecure = value in ["1", "true", "TRUE", "TrUe"]
                if insecure:
                    # Both device authorization requests reuse the process-wide agent.
                    passed = count == 2 and warnings == 1 and "User code expired" in run.stderr
                else:
                    passed = count == 0 and warnings == 0 and (
                        "invalid peer certificate" in run.stderr or "TLS" in run.stderr
                    )
                if not passed:
                    raise AssertionError((value, count, warnings, run.stderr))
                results.append({"value": value, "http_requests": count,
                                "warnings": warnings})
        finally:
            server.shutdown()
            server.server_close()
            thread.join()
        print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
