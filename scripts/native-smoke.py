"""Launch the real WebView and drive its controls through the smoke-test feature."""
import os
import json
import shutil
import pathlib
import subprocess
import threading

# Other CI jobs still fetch assets; only the install job exercises acquisition.
assets = pathlib.Path(os.environ.get('VZ_SMOKE_DATA_ROOT', pathlib.Path.home() / 'vectorAIz')) / 'models'
assets.mkdir(parents=True, exist_ok=True)
for pin in json.loads(pathlib.Path('spike/fixtures/models.json').read_text()).values():
    shutil.copyfile(pathlib.Path('.assets') / pin['file'], assets / pin['file'])
os.environ['VZ_PIPELINE_FIXTURE'] = str(pathlib.Path('spike/fixtures/documents').resolve())

binary = pathlib.Path('target/debug/vectoraiz.exe' if __import__('sys').platform == 'win32' else 'target/debug/vectoraiz')

def run(mode, marker, timeout):
    lines = []
    # Hosted runners exercise the full pipeline on CPU, including UI IPC.
    child = subprocess.Popen([str(binary.resolve()), mode, '--cpu'], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding="utf-8", bufsize=1)
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
