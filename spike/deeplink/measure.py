#!/usr/bin/env python3
"""Deep-link length probe (Phase 0 chunk F, spec 4.6).
Serves a page on 127.0.0.1 that reports the received #vz= fragment length and sha256,
then opens URLs of growing size through macOS LaunchServices (`open -a <browser> <url>`),
a LaunchServices transport proxy for the app's future opener (equivalence unverified). Also tests the local-HTML location.replace fallback.
Usage: measure.py "Google Chrome" [--fallback]"""
import base64, hashlib, http.server, json, os, secrets, subprocess, sys, tempfile, threading, time
SIZES = [2_000, 8_000, 16_000, 32_000, 48_000, 64_000, 128_000, 256_000, 1_000_000]
got = {}
PAGE = b"""<!doctype html><meta charset=utf-8><title>vz probe</title><script>
const h=location.hash, f=h.startsWith('#vz=')?h.slice(4):'';
crypto.subtle.digest('SHA-256',new TextEncoder().encode(f)).then(d=>{
 const hex=[...new Uint8Array(d)].map(b=>b.toString(16).padStart(2,'0')).join('');
 fetch('/report',{method:'POST',body:JSON.stringify({id:new URLSearchParams(location.search).get('id'),len:f.length,sha:hex,href_len:location.href.length})})
 .then(()=>{document.body.textContent='reported '+f.length; setTimeout(()=>window.close(),300)});});
</script><body>probing</body>"""
class H(http.server.BaseHTTPRequestHandler):
    def log_message(self,*a): pass
    def do_GET(self):
        self.send_response(200); self.send_header("Content-Type","text/html"); self.end_headers(); self.wfile.write(PAGE)
    def do_POST(self):
        d=json.loads(self.rfile.read(int(self.headers["Content-Length"]))); got[d["id"]]=d
        self.send_response(204); self.end_headers()
srv=http.server.ThreadingHTTPServer(("127.0.0.1",0),H); port=srv.server_address[1]
threading.Thread(target=srv.serve_forever,daemon=True).start()
browser=sys.argv[1]; fallback="--fallback" in sys.argv
results=[]
for n in SIZES:
    frag=base64.urlsafe_b64encode(secrets.token_bytes(n*3//4+3)).decode().rstrip("=")[:n]
    want=hashlib.sha256(frag.encode()).hexdigest(); rid=f"{n}-{secrets.token_hex(3)}"
    url=f"http://127.0.0.1:{port}/?id={rid}#vz={frag}"
    tmp=None
    if fallback:
        fd,tmp=tempfile.mkstemp(suffix=".html"); os.write(fd,("<!doctype html><script>location.replace("+json.dumps(url)+")</script>").encode()); os.close(fd)
        target=tmp
    else:
        target=url
    try:
        t0=time.time(); p=subprocess.run(["open","-a",browser,target],capture_output=True,text=True)
        while rid not in got and time.time()-t0<20: time.sleep(0.2)
        r=got.get(rid); ok=bool(r and r["len"]==n and r["sha"]==want)
        results.append({"size":n,"open_rc":p.returncode,"open_err":p.stderr.strip()[:120],"received":r["len"] if r else None,"intact":ok})
        print(json.dumps(results[-1]),flush=True)
        if tmp: time.sleep(1)
    finally:
        if tmp and os.path.exists(tmp): os.unlink(tmp)
print(json.dumps({"browser":browser,"fallback":fallback,"results":results}))
