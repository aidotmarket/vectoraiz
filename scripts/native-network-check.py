#!/usr/bin/env python3
"""Provider transport fence; CLI and pack exports are explicitly out of scope."""
from pathlib import Path
import re
import sys
ROOT = Path(__file__).resolve().parents[1]
CONSTRUCTORS = re.compile(r"Client::new\(|Client::builder\(|ClientBuilder::new\(|reqwest::blocking")
allowed = {"crates/vz-core/src/cloud_gate.rs", "app/src-tauri/src/models.rs", "app/src-tauri/src/models/tests.rs"}
errors = []
for base in ["app/src-tauri/src", "crates/vz-core/src"]:
    for path in (ROOT / base).rglob("*.rs"):
        relative = path.relative_to(ROOT).as_posix()
        for number, line in enumerate(path.read_text().splitlines(), 1):
            if CONSTRUCTORS.search(line) and relative not in allowed:
                errors.append(f"{relative}:{number}: HTTP client outside transport boundary")
            if re.search(r"anthropic\.com|openai\.com", line) and relative != "crates/vz-core/src/cloud_gate.rs":
                errors.append(f"{relative}:{number}: provider destination outside transport boundary")
# Local MCP code may never acquire an Internet transport. The installed-test
# negative control is confined to the existing smoke-test checkpoint in desktop.rs.
for path in [*(ROOT / "crates/vz-mcp/src").rglob("*.rs"),
             *(ROOT / "app/src-tauri/src").glob("mcp_*.rs")]:
    for number, line in enumerate(path.read_text().splitlines(), 1):
        if re.search(r"reqwest|TcpStream|TcpListener|UdpSocket|std::net|tokio::net::(tcp|udp)", line):
            errors.append(f"{path.relative_to(ROOT)}:{number}: Internet transport in local MCP boundary")
# UI destinations are confined to the reviewed copy list in App.tsx.
for path in (ROOT / "app/src").rglob("*"):
    if path.is_file() and path.suffix in {".ts", ".tsx"} and path.name != "App.tsx":
        if re.search(r"anthropic\.com|openai\.com", path.read_text()):
            errors.append(f"{path.relative_to(ROOT)}: provider destination outside UI strings file")
if errors:
    print("\n".join(errors), file=sys.stderr)
    sys.exit(1)
print("Provider network boundary: PASS (desktop + vz-core; CLI/pack export paths excluded)")
