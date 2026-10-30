"""Loopback TLS fixture for Rust acquisition tests; generates disposable keys.

Uses Python's TLS server and the OpenSSL already supplied by macOS/Git for Windows.
No certificate, key or trust override is shipped in the application.
"""
import http.server
import json
from pathlib import Path
import shutil
import ssl
import socketserver
import subprocess
import sys
import time


class LoopbackServer(http.server.ThreadingHTTPServer):
    def server_bind(self):
        # HTTPServer.server_bind calls socket.getfqdn(), which can wait for
        # reverse-DNS timeouts even for 127.0.0.1 on hosted macOS runners.
        # This fixture only needs the numeric bound port, never a DNS name.
        socketserver.TCPServer.server_bind(self)
        self.server_name = 'localhost'
        self.server_port = self.socket.getsockname()[1]


def main():
    started = time.monotonic()
    root = Path(sys.argv[1])
    openssl = shutil.which('openssl')
    if not openssl and sys.platform == 'win32':
        openssl = str(Path('C:/Program Files/Git/usr/bin/openssl.exe'))
    config = root / 'openssl.cnf'
    config.write_text('[req]\ndistinguished_name=dn\nx509_extensions=ext\nprompt=no\n'
                      '[dn]\nCN=localhost\n[ext]\nsubjectAltName=DNS:localhost,IP:127.0.0.1\n'
                      'basicConstraints=critical,CA:TRUE\nkeyUsage=digitalSignature,keyEncipherment,keyCertSign\nextendedKeyUsage=serverAuth\n')
    subprocess.run([openssl, 'req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '1',
                    '-config', str(config), '-keyout', str(root / 'key.pem'),
                    '-out', str(root / 'cert.pem')], check=True, stdout=subprocess.DEVNULL,
                   stderr=subprocess.DEVNULL)
    print(f'TLS fixture certificate ready in {time.monotonic() - started:.3f}s', file=sys.stderr, flush=True)
    plan = json.loads((root / 'plan.json').read_text())
    requests = []

    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_GET(self):
            index = len(requests)
            requests.append({'path': self.path, 'range': self.headers.get('Range')})
            (root / 'requests.json').write_text(json.dumps(requests))
            action = plan[min(index, len(plan) - 1)]
            time.sleep(action.get('header_delay', 0))
            try:
                self.send_response(action.get('status', 200))
                for key, value in action.get('headers', {}).items():
                    self.send_header(key, value.replace('{port}', str(self.server.server_port)))
                body = action.get('byte', 'x').encode() * action.get('length', 100)
                self.send_header('Content-Length', str(action.get('declared_length', len(body))))
                self.end_headers()
                prefix = action.get('prefix', 0)
                if prefix:
                    self.wfile.write(body[:prefix])
                    self.wfile.flush()
                time.sleep(action.get('body_delay', 0))
                self.wfile.write(body[prefix:])
            except (BrokenPipeError, ConnectionError, ssl.SSLError):
                pass

    server = LoopbackServer(('127.0.0.1', 0), Handler)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(root / 'cert.pem', root / 'key.pem')
    server.socket = context.wrap_socket(server.socket, server_side=True)
    print(f'TLS fixture listener ready in {time.monotonic() - started:.3f}s', file=sys.stderr, flush=True)
    print(server.server_port, flush=True)
    server.serve_forever()


if __name__ == '__main__':
    main()
