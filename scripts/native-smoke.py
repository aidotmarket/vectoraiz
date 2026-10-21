"""Launch the debug WebView and click the actual self-test button via the test feature."""
import pathlib
import subprocess
import sys

binary = pathlib.Path("target/debug/vectoraiz" + (".exe" if sys.platform == "win32" else ""))
result = subprocess.run(
    [str(binary.resolve()), "--smoke-test"],
    stdout=subprocess.PIPE,
    stderr=subprocess.STDOUT,
    text=True,
    timeout=60,
    check=True,
)
print(result.stdout)
assert "VECTORAIZ_SMOKE_TEST_OK" in result.stdout, "WebView did not invoke the self-test"

pipeline = subprocess.run(
    [str(binary.resolve()), "--pipeline-smoke-test"],
    stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=300, check=True,
)
print(pipeline.stdout)
assert "VECTORAIZ_PIPELINE_SMOKE_TEST_OK" in pipeline.stdout, "WebView pipeline did not complete"
