"""Double-launch the real desktop, including with the first window minimized."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time


def wait_report(path, child):
    deadline = time.monotonic() + 30
    while not path.exists() and time.monotonic() < deadline:
        assert child.poll() is None, 'First instance exited unexpectedly'
        time.sleep(0.1)
    assert path.is_file(), f'No window report: {path}'
    return json.loads(path.read_text())


def test(binary, cwd=None, env=None):
    results = []
    for minimized in (False, True):
        with tempfile.TemporaryDirectory(prefix='vz-single-instance-') as temporary:
            root = Path(temporary)
            ready, report = root / 'ready.json', root / 'focus.json'
            settings = dict(env or os.environ, VZ_SINGLE_INSTANCE_READY=str(ready),
                            VZ_SINGLE_INSTANCE_REPORT=str(report))
            if minimized:
                settings['VZ_SINGLE_INSTANCE_MINIMIZE'] = '1'
            first = subprocess.Popen([str(binary), '--single-instance-test'], cwd=cwd, env=settings)
            try:
                initial = wait_report(ready, first)
                assert initial['pid'] == first.pid and initial['minimized'] == minimized, initial
                second = subprocess.Popen([str(binary), '--single-instance-test'], cwd=cwd, env=settings)
                try:
                    assert second.wait(timeout=20) == 0, 'Second launch did not exit'
                finally:
                    if second.poll() is None:
                        second.kill()
                        second.wait(timeout=10)
                focused = wait_report(report, first)
                assert focused == {'pid': first.pid, 'minimized': False, 'focused': True}, focused
                assert first.poll() is None, 'Original process must remain alive'
                results.append({'initial': initial, 'focused': focused, 'second_exit': second.returncode})
                print(f'PASS: second launch exited, original pid {first.pid} focused; initially minimized={minimized}', flush=True)
            finally:
                first.terminate()
                try:
                    first.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    first.kill()
                    first.wait(timeout=10)

    return results


if __name__ == '__main__':
    test(Path(sys.argv[1]).resolve())
