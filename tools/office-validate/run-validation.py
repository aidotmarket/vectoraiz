"""Run both independent checks and preserve failures in one JSON artifact."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile

LIBREOFFICE_VERSION = "26.2.6"
EXTENSIONS = {".docx", ".xlsx", ".pptx"}


def run(command, timeout):
    process = subprocess.Popen(
        [str(arg) for arg in command], stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        text=True, encoding="utf-8", errors="replace", start_new_session=os.name != "nt",
    )
    timed_out = False
    try:
        stdout, stderr = process.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        timed_out = True
        # soffice launches a child process. Kill the entire tree before continuing.
        if os.name == "nt":
            subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"],
                           capture_output=True, timeout=15, check=False)
        else:
            os.killpg(process.pid, signal.SIGKILL)
        stdout, stderr = process.communicate(timeout=15)
    return {"exitCode": process.returncode, "timedOut": timed_out,
            "stdout": stdout, "stderr": stderr}


def render(path, root, soffice, version_ok, timeout):
    result = {"path": path.relative_to(root).as_posix(),
              "sha256": None, "ok": False, "pageCount": None}
    try:
        result["sha256"] = hashlib.sha256(path.read_bytes()).hexdigest()
        # Isolate profiles/output directories for duplicate stems and stale PDFs.
        with tempfile.TemporaryDirectory(prefix="office-render-") as temp:
            temp = Path(temp)
            conversion = run([
                soffice, f"-env:UserInstallation={(temp / 'profile').as_uri()}",
                "--headless", "--nologo", "--nodefault", "--norestore",
                "--convert-to", "pdf", "--outdir", temp, path,
            ], timeout)
            result.update(conversion)
            if conversion["timedOut"] or conversion["exitCode"] != 0:
                raise RuntimeError("LibreOffice conversion failed or timed out")
            pdf = temp / (path.stem + ".pdf")
            if not pdf.is_file() or pdf.stat().st_size == 0:
                raise RuntimeError("LibreOffice produced no nonempty PDF (even if exit code was zero)")
            from pypdf import PdfReader

            result["pageCount"] = len(PdfReader(pdf).pages)
            if result["pageCount"] == 0:
                raise RuntimeError("PDF contains no pages")
            if not version_ok:
                raise RuntimeError("LibreOffice version did not match the pin")
            result["ok"] = True
    except Exception as error:
        result["error"] = f"{type(error).__name__}: {error}"
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--sdk-dll", type=Path, required=True)
    parser.add_argument("--soffice", required=True)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--render-timeout", type=int, default=60)
    args = parser.parse_args()
    root = args.directory.resolve()
    report = {"ok": False, "note": "LibreOffice is not Microsoft Office.",
              "libreOfficeVersionPin": LIBREOFFICE_VERSION,
              "openXml": None, "libreOffice": [], "errors": []}
    try:
        if not root.is_dir():
            raise FileNotFoundError(root)
        files = sorted(p for p in root.rglob("*") if p.is_file() and p.suffix.lower() in EXTENSIONS)
        if not files:
            report["errors"].append("No DOCX/XLSX/PPTX inputs found")
        try:
            sdk = run(["dotnet", args.sdk_dll.resolve(), root], 300)
            report["sdkProcess"] = sdk
            report["openXml"] = json.loads(sdk["stdout"])
            if sdk["exitCode"] != 0 or sdk["timedOut"] or not report["openXml"]["ok"]:
                report["errors"].append("Open XML SDK validation failed")
            expected = {p.relative_to(root).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest()
                        for p in files}
            actual = {f["path"]: f["sha256"] for f in report["openXml"]["files"]}
            if actual != expected:
                report["errors"].append("SDK report does not match the input files/hashes")
        except Exception as error:
            report["errors"].append(f"Open XML SDK: {type(error).__name__}: {error}")
        version_ok = False
        try:
            version = run([args.soffice, "--version"], 30)
            report["libreOfficeVersion"] = version
            version_ok = (version["exitCode"] == 0 and not version["timedOut"] and
                          version["stdout"].strip().startswith(f"LibreOffice {LIBREOFFICE_VERSION}."))
            if not version_ok:
                report["errors"].append("LibreOffice version check failed")
        except Exception as error:
            report["errors"].append(f"LibreOffice version: {type(error).__name__}: {error}")
        report["libreOffice"] = [render(p, root, args.soffice, version_ok, args.render_timeout)
                                 for p in files]
        report["ok"] = (bool(files) and not report["errors"] and
                        all(item["ok"] for item in report["libreOffice"]))
    except Exception as error:
        report["errors"].append(f"{type(error).__name__}: {error}")
    finally:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(f"Office validation: ok={report['ok']}; report={args.report}")
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
