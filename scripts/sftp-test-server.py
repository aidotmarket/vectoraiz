#!/usr/bin/env python3
"""Loopback-only SFTP acceptance fixture. Requires paramiko==3.5.1.

Never use this server with real data: it accepts the generated fixture user's key.
All files and disposable private keys stay in the test's temporary directory.
"""
import argparse
import errno
import json
import os
from pathlib import Path
import socket
import threading
import time

import paramiko

parser = argparse.ArgumentParser()
parser.add_argument("root", type=Path)
parser.add_argument("--port", type=int, default=0)
parser.add_argument("--host-key", default="host-a.pem")
args = parser.parse_args()
root = args.root.resolve()


def key_file(name):
    path = root / name
    if not path.exists():
        paramiko.RSAKey.generate(2048).write_private_key_file(str(path))
    return paramiko.RSAKey.from_private_key_file(str(path))


host_key = key_file(args.host_key)
client_key = key_file("client.pem")
client_key.write_private_key_file(str(root / "encrypted.pem"), password="fixture-only")


class Auth(paramiko.ServerInterface):
    def get_allowed_auths(self, username):
        return "publickey"

    def check_auth_publickey(self, username, key):
        return (paramiko.AUTH_SUCCESSFUL if username == "fixture" and key == client_key
                else paramiko.AUTH_FAILED)

    def check_channel_request(self, kind, channel_id):
        return (paramiko.OPEN_SUCCEEDED if kind == "session"
                else paramiko.OPEN_FAILED_ADMINISTRATIVELY_PROHIBITED)


class Sftp(paramiko.SFTPServerInterface):
    def __init__(self, server, *args, **kwargs):
        self.transport = kwargs.pop("transport")
        super().__init__(server, *args, **kwargs)

    def local(self, path):
        result = (root / path.lstrip("/")).resolve()
        if not result.is_relative_to(root):
            raise OSError(errno.EACCES, "outside fixture")
        return result

    def canonicalize(self, path):
        return "/" + self.local(path).relative_to(root).as_posix()

    def stat(self, path):
        try:
            attr = paramiko.SFTPAttributes.from_stat(self.local(path).stat())
            if (root / "bad-size").exists() and ".partial-" in path:
                attr.st_size += 1
            return attr
        except OSError as error:
            return paramiko.SFTPServer.convert_errno(error.errno)

    lstat = stat

    def mkdir(self, path, attr):
        try:
            self.local(path).mkdir()
            return paramiko.SFTP_OK
        except OSError as error:
            return paramiko.SFTPServer.convert_errno(error.errno)

    def open(self, path, flags, attr):
        try:
            descriptor = os.open(self.local(path), flags, 0o600)
            file = os.fdopen(descriptor, "r+b" if flags & os.O_RDWR else
                             "wb" if flags & os.O_WRONLY else "rb")
            class Handle(paramiko.SFTPHandle):
                def read(self, offset, length):
                    data = super().read(offset, length)
                    if (root / "bad-hash").exists() and isinstance(data, bytes) and data:
                        return bytes([data[0] ^ 1]) + data[1:]
                    return data

            handle = Handle(flags)
            handle.readfile = file
            handle.writefile = file
            return handle
        except OSError as error:
            return paramiko.SFTPServer.convert_errno(error.errno)

    def rename(self, old, new):
        if (root / "fail-rename").exists():
            return paramiko.SFTP_FAILURE
        try:
            if self.local(new).exists():
                return paramiko.SFTP_FAILURE
            self.local(old).rename(self.local(new))
            if (root / "lost-rename-ack").exists():
                # Commit, then close before Paramiko can send the RENAME status.
                self.transport.close()
            return paramiko.SFTP_OK
        except OSError as error:
            return paramiko.SFTPServer.convert_errno(error.errno)


def serve(connection):
    transport = paramiko.Transport(connection)
    transport.add_server_key(host_key)
    transport.set_subsystem_handler("sftp", paramiko.SFTPServer, Sftp, transport=transport)
    try:
        transport.start_server(server=Auth())
        while transport.is_active():
            time.sleep(0.02)
    finally:
        transport.close()


listener = socket.socket()
listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
listener.bind(("127.0.0.1", args.port))
listener.listen(16)
print(json.dumps({"port": listener.getsockname()[1]}), flush=True)
while True:
    connection, _ = listener.accept()
    threading.Thread(target=serve, args=(connection,), daemon=True).start()
