package main

import (
	"path/filepath"
	"testing"
	"time"
)

// Also run unchanged against the exact 098c process/OS adapters via a Go source
// overlay. This uses only a temporary stdlib file lock, never a native child.
func TestRegressionDurableDoneRequiresReleasedLock(t *testing.T) {
	if allowNativeExecution {
		t.Fatal("native execution guard open")
	}
	root := t.TempDir()
	f, e := lock(filepath.Join(root, "child.lock"))
	if e != nil {
		t.Fatal(e)
	}
	locked := true
	defer func() {
		if locked {
			unlock(f)
		}
	}()
	r := Command{ID: token(), Started: true, Done: true, Deadline: time.Now().Unix() + 30, Code: "command_exit"}
	if e := save(filepath.Join(root, "child.json"), r); e != nil {
		t.Fatal(e)
	}
	result := make(chan error, 1)
	p := Processes{Root: root}
	go func() { result <- p.Reconcile() }()
	select {
	case e := <-result:
		t.Fatalf("durable Done accepted while predecessor still owns child.lock: %v", e)
	case <-time.After(30 * time.Millisecond):
	}
	unlock(f)
	locked = false
	select {
	case e := <-result:
		if e != nil {
			t.Fatal(e)
		}
	case <-time.After(time.Second):
		t.Fatal("reconciliation did not finish after release")
	}
}
