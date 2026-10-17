// Standalone test glue. Compile ONLY with the three byte-exact production files.
// This is not a production owner executable or an installation entry point.
package main

import (
 "encoding/json"
 "errors"
 "os"
 "path/filepath"
 "strconv"
 "time"
)

type probeResult struct {
 Operation string
 Output []byte
 Error string
 FinishedUnixNano int64
}

func probe() error {
 a := os.Args
 // Preserve the product helper/watchdog argv and real dispatch.
 if len(a) == 4 && a[1] == "command" { return commandMain(a[2], a[3]) }
 if len(a) == 4 && a[1] == "watchdog" {
  deadline, e := strconv.ParseInt(a[2], 10, 64); if e != nil { return e }
  group, e := strconv.Atoi(a[3]); if e != nil { return e }
  return watchdogMain(deadline, group)
 }
 if len(a) != 5 || (a[1] != "run" && a[1] != "reconcile") { return errors.New("probe_arguments") }
 root, e := filepath.Abs(a[2]); if e != nil { return e }
 if root != a[2] { return errors.New("probe_absolute_root") }
 if e = secureDirectory(root); e != nil { return e }
 deadline, e := strconv.ParseInt(a[3], 10, 64); if e != nil { return e }
 if deadline <= time.Now().Unix() || deadline > time.Now().Unix()+20 { return errors.New("probe_original_budget") }
 p := Processes{Root: root, Cwd: root, Env: []string{"R374_SYNTHETIC=1"}}
 result := probeResult{Operation: a[1]}
 if a[1] == "run" {
  if _, e = os.Lstat(filepath.Join(root, "child.json")); !os.IsNotExist(e) { return errors.New("probe_fresh_run_required") }
  result.Output, e = p.Run([]string{a[4], filepath.Join(root, "work.py")}, []byte("synthetic-input"), deadline, 4096, true)
 } else {
  var original Command
  if e = load(filepath.Join(root, "child.json"), &original); e != nil { return e }
  if original.Deadline != deadline { return errors.New("probe_deadline_changed") }
  if e = atomicFile(filepath.Join(root, "reconcile-entered"), []byte("entered")); e != nil { return e }
  e = p.Reconcile()
 }
 if e != nil { result.Error = e.Error() }
 result.FinishedUnixNano = time.Now().UnixNano()
 b, e := json.Marshal(result); if e != nil { return e }
 return atomicFile(filepath.Join(root, a[1]+"-result.json"), b)
}

func main() {
 if e := probe(); e != nil { os.Stderr.WriteString(e.Error()+"\n"); os.Exit(2) }
}
