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
    while time.monotonic() < deadline:
        assert child.poll() is None, 'First instance exited unexpectedly'
        if path.is_file():
            try:
                return json.loads(path.read_text())
            except json.JSONDecodeError:
                pass  # The writer may still be completing the report.
        time.sleep(0.1)
    raise AssertionError(f'No complete window report: {path}')


def desktop_evidence():
    if sys.platform != 'win32':
        return {'focus_required': True}
    import ctypes
    from ctypes import wintypes
    user32 = ctypes.WinDLL('user32', use_last_error=True)
    kernel32 = ctypes.WinDLL('kernel32', use_last_error=True)
    kernel32.GetCurrentThreadId.restype = wintypes.DWORD
    user32.GetThreadDesktop.argtypes = [wintypes.DWORD]
    user32.GetThreadDesktop.restype = wintypes.HANDLE
    user32.GetUserObjectInformationW.argtypes = [wintypes.HANDLE, ctypes.c_int,
                                               wintypes.LPVOID, wintypes.DWORD,
                                               ctypes.POINTER(wintypes.DWORD)]
    user32.GetUserObjectInformationW.restype = wintypes.BOOL
    user32.GetForegroundWindow.restype = wintypes.HWND
    receiving_input = wintypes.BOOL()
    desktop = user32.GetThreadDesktop(kernel32.GetCurrentThreadId())
    queried = bool(desktop and user32.GetUserObjectInformationW(
        desktop, 6, ctypes.byref(receiving_input), ctypes.sizeof(receiving_input), None))  # UOI_IO
    hosted = (os.environ.get('GITHUB_ACTIONS') == 'true'
              and os.environ.get('RUNNER_ENVIRONMENT') == 'github-hosted')
    # Only a positively identified non-input desktop on a hosted runner permits
    # reduced assertions. Unknown/query failure and all interactive runs stay strict.
    return {'focus_required': not (hosted and queried and not receiving_input.value),
            'github_hosted': hosted, 'input_desktop_query_ok': queried,
            'receiving_input': bool(receiving_input.value) if queried else None,
            'foreground_hwnd': user32.GetForegroundWindow()}


def test(binary, cwd=None, env=None, result_path=None):
    results = []
    for minimized in (False, True):
        with tempfile.TemporaryDirectory(prefix='vz-single-instance-') as temporary:
            root = Path(temporary)
            ready, report = root / 'ready.json', root / 'focus.json'
            handoff = root / 'handoff.json'
            desktop = desktop_evidence()
            settings = dict(env or os.environ, VZ_SINGLE_INSTANCE_READY=str(ready),
                            VZ_SINGLE_INSTANCE_REPORT=str(report),
                            VZ_SINGLE_INSTANCE_HANDOFF=str(handoff))
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
                permission = json.loads(handoff.read_text()) if sys.platform == 'win32' else None
                desktop_after = desktop_evidence()
                focus_required = desktop['focus_required'] or desktop_after['focus_required']
                result = {'initial': initial, 'focused': focused, 'second_exit': second.returncode,
                          'original_alive': first.poll() is None, 'desktop_before': desktop,
                          'desktop_after': desktop_after, 'handoff': permission,
                          'real_focus': 'VERIFIED' if focus_required and focused['focused'] else 'UNVERIFIED'}
                results.append(result)
                if result_path:
                    Path(result_path).write_text(json.dumps(results, indent=2))
                print(json.dumps(result), flush=True)
                assert focused['pid'] == first.pid and focused['minimized'] is False, focused
                assert focused['visible'] is True, focused
                assert focused['focus_requested'] is True and focused['focus_request_accepted'] is True, focused
                assert first.poll() is None, 'Original process must remain alive'
                if permission is not None:
                    assert permission['pid'] == second.pid and permission['target_pid'] == first.pid, permission
                if focus_required:
                    assert focused['focused'] is True, focused
                print(f'PASS: second exit 0, original pid {first.pid} alive, visible, restored, focus requested; '
                      f"real focus {result['real_focus']}; initially minimized={minimized}", flush=True)
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
