#!/usr/bin/env python3
"""Install checksum-pinned protoc without querying the GitHub releases API."""
import hashlib
import os
from pathlib import Path
import platform
import tempfile
import time
import urllib.error
import urllib.request
import zipfile

VERSION = '29.3'
# SHA-256 of the exact upstream v29.3 release archives, not a mutable latest URL.
ARCHIVES = {
    ('Darwin', 'arm64'): ('osx-aarch_64', '2b8a3403cd097f95f3ba656e14b76c732b6b26d7f183330b11e36ef2bc028765'),
    ('Darwin', 'x86_64'): ('osx-x86_64', '9a788036d8f9854f7b03c305df4777cf0e54e5b081e25bf15252da87e0e90875'),
    ('Windows', 'amd64'): ('win64', '57ea59e9f551ad8d71ffaa9b5cfbe0ca1f4e720972a1db7ec2d12ab44bff9383'),
}


def download(url, archive):
    for attempt in range(4):
        try:
            with urllib.request.urlopen(url, timeout=60) as response, archive.open('wb') as output:
                while chunk := response.read(1024 * 1024):
                    output.write(chunk)
            return
        except (OSError, urllib.error.URLError):
            if attempt == 3:
                raise
            print(f'protoc download failed; retry {attempt + 1}/3', flush=True)
            time.sleep(2 ** (attempt + 1))


def install(destination, system, machine):
    try:
        target, expected = ARCHIVES[(system, machine.lower())]
    except KeyError:
        raise SystemExit(f'Unsupported protoc platform: {system}/{machine}')
    name = f'protoc-{VERSION}-{target}.zip'
    url = f'https://github.com/protocolbuffers/protobuf/releases/download/v{VERSION}/{name}'
    with tempfile.TemporaryDirectory(prefix='protoc-download-') as temporary:
        archive = Path(temporary) / name
        download(url, archive)
        actual = hashlib.sha256(archive.read_bytes()).hexdigest()
        if actual != expected:
            raise SystemExit(f'{name}: SHA-256 mismatch: expected {expected}, got {actual}')
        with zipfile.ZipFile(archive) as zipped:
            zipped.extractall(destination)
    executable = destination / 'bin' / ('protoc.exe' if system == 'Windows' else 'protoc')
    executable.chmod(0o755)
    return executable


def main():
    destination = Path(os.environ['RUNNER_TEMP']) / f'protoc-{VERSION}'
    executable = install(destination, platform.system(), platform.machine())
    with Path(os.environ['GITHUB_PATH']).open('a', encoding='utf-8') as output:
        output.write(str(executable.parent) + '\n')
    print(f'Installed checksum-verified protoc {VERSION}: {executable}', flush=True)


if __name__ == '__main__':
    main()
