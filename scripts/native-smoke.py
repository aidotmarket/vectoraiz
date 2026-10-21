"""Launch the real WebView and drive its controls through the smoke-test feature."""
import pathlib
import subprocess
import threading

binary = pathlib.Path('target/debug/vectoraiz.exe' if __import__('sys').platform == 'win32' else 'target/debug/vectoraiz')

def run(mode, marker, timeout):
    lines = []
    child = subprocess.Popen([str(binary.resolve()), mode], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1)
    def capture():
        for line in child.stdout:
            lines.append(line)
            print(line, end='', flush=True)
    reader = threading.Thread(target=capture, daemon=True)
    reader.start()
    try:
        code = child.wait(timeout=timeout)
    finally:
        if child.poll() is None:
            child.kill(); child.wait()
        reader.join(timeout=5)
    assert code == 0, f'{mode} exited {code}'
    assert marker in ''.join(lines), f'{mode} did not complete in the WebView'

run('--smoke-test', 'VECTORAIZ_SMOKE_TEST_OK', 60)
run('--pipeline-smoke-test', 'VECTORAIZ_PIPELINE_SMOKE_TEST_OK', 300)
