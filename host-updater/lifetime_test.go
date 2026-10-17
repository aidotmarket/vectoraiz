package main

import (
	"errors"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"
)

// These are in-memory locks and simulated children, never native fixtures.
type memoryLocks struct {
	mu      sync.Mutex
	held    map[string]bool
	blocked chan string
}

func (m *memoryLocks) acquire(path string) (func(), error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.held[path] {
		select {
		case m.blocked <- filepath.Base(path):
		default:
		}
		return nil, errors.New("installation_busy")
	}
	m.held[path] = true
	return func() { m.mu.Lock(); delete(m.held, path); m.mu.Unlock() }, nil
}
func newMemoryLocks() *memoryLocks {
	return &memoryLocks{held: map[string]bool{}, blocked: make(chan string, 32)}
}

func TestLifetimeDoneBeforeUnlockSerializesNextCommand(t *testing.T) {
	if allowNativeExecution {
		t.Fatal("native guard open")
	}
	root := t.TempDir()
	locks := newMemoryLocks()
	aDone, unlockA := make(chan struct{}), make(chan struct{})
	p := Processes{Root: root, Acquire: locks.acquire, Pause: func() { time.Sleep(time.Millisecond) }}
	var mu sync.Mutex
	calls := map[string]int{}
	p.StartCommand = func(r Command) error {
		release, e := locks.acquire(filepath.Join(root, "child.lock"))
		if e != nil {
			t.Error("B started before A released ownership")
			return e
		}
		mu.Lock()
		calls[r.Args[0]]++
		mu.Unlock()
		name := r.Args[0]
		r.Started, r.Done, r.Output = true, true, []byte(name)
		r.Args, r.Env, r.Input = nil, nil, nil
		if e := save(filepath.Join(root, "child.json"), &r); e != nil {
			release()
			return e
		}
		if name == "A" {
			close(aDone)
			go func() { <-unlockA; release() }()
		} else {
			release()
		}
		return nil
	}
	results := make(chan error, 2)
	run := func(name string) {
		out, e := p.Run([]string{name}, nil, time.Now().Unix()+30, 32, true)
		if e == nil && string(out) != name {
			e = errors.New("wrong durable output")
		}
		results <- e
	}
	go run("A")
	awaitFake(t, aDone)
	go run("B")
	// A waits for child.lock; B must wait for handoff.lock without overwriting A.
	wait := time.After(time.Second)
	for {
		select {
		case name := <-locks.blocked:
			if name == "handoff.lock" {
				goto blocked
			}
		case <-wait:
			t.Fatal("B never reached serialized handoff")
		}
	}
blocked:
	var got Command
	if e := load(filepath.Join(root, "child.json"), &got); e != nil || string(got.Output) != "A" {
		t.Fatal("B overwrote completed A before unlock", e)
	}
	select {
	case e := <-results:
		t.Fatal("returned before unlock", e)
	default:
	}
	close(unlockA)
	for n := 0; n < 2; n++ {
		select {
		case e := <-results:
			if e != nil {
				t.Fatal(e)
			}
		case <-time.After(time.Second):
			t.Fatal("artificial timeout after release")
		}
	}
	mu.Lock()
	defer mu.Unlock()
	if calls["A"] != 1 || calls["B"] != 1 {
		t.Fatal("command duplicated/lost", calls)
	}
}

func TestLifetimeReconcileCompletedStillWaitsForUnlock(t *testing.T) {
	root := t.TempDir()
	locks := newMemoryLocks()
	release, _ := locks.acquire(filepath.Join(root, "child.lock"))
	r := Command{ID: token(), Started: true, Done: true, Deadline: 1000, Code: "command_exit", Output: []byte("durable")}
	if e := save(filepath.Join(root, "child.json"), r); e != nil {
		t.Fatal(e)
	}
	paused := 0
	p := Processes{Root: root, Acquire: locks.acquire, Now: func() time.Time { return time.Unix(900, 0) }, Pause: func() { paused++; release() }}
	if e := p.Reconcile(); e != nil {
		t.Fatal(e)
	}
	var got Command
	load(filepath.Join(root, "child.json"), &got)
	if paused != 1 || got.Code != r.Code || string(got.Output) != "durable" {
		t.Fatal("completion accepted before ownership release or overwritten", paused, got)
	}
}

func TestLifetimeHelperDeathAndDeadlineAcrossOwnerRestart(t *testing.T) {
	for _, trigger := range []string{"helper-death", "deadline"} {
		t.Run(trigger, func(t *testing.T) {
			root := t.TempDir()
			now := int64(100)
			locks := newMemoryLocks()
			r := Command{ID: token(), Started: true, ProcessGroup: 12345, Deadline: 105, Args: []string{"original-context-activation"}, Env: []string{"SECRET=fake"}}
			if e := save(filepath.Join(root, "child.json"), r); e != nil {
				t.Fatal(e)
			}
			alive := true
			stops := 0
			ended := make(chan struct{})
			deadline := make(chan time.Time, 1)
			if trigger == "helper-death" {
				close(ended)
			} else {
				deadline <- time.Unix(r.Deadline, 0)
			}
			// Baseline counterexample: losing the helper loses its context callback;
			// a restarted owner only observes alive=true, indefinitely (no stop action).
			baseline := Processes{Root: root, Acquire: locks.acquire, Now: func() time.Time { return time.Unix(111, 0) }, GroupAlive: func(int) bool { return alive }}
			if e := baseline.Reconcile(); e == nil || e.Error() != "child_may_be_running" {
				t.Fatal("baseline refusal counterexample missing", e)
			}
			watchLifetime(ended, deadline, func() { stops++; alive = false })
			// A new polling owner has no timer/identity to kill. The independent watcher
			// already bounded the descendants, using the original absolute deadline.
			now = 111
			restarted := Processes{Root: root, Acquire: locks.acquire, Now: func() time.Time { return time.Unix(now, 0) }, GroupAlive: func(pid int) bool {
				if pid != r.ProcessGroup {
					t.Fatal("wrong identity")
				}
				return alive
			}}
			if e := restarted.Reconcile(); e == nil || e.Error() != "child_outcome_unknown" {
				t.Fatal("lost mutation declared success or remained blocked", e)
			}
			var got Command
			load(filepath.Join(root, "child.json"), &got)
			if stops != 1 || !got.Done || got.Code != "child_outcome_unknown" || got.Args != nil || got.Env != nil || got.Deadline != 105 || got.ID != r.ID {
				t.Fatal("lost durable uncertainty/original budget", got)
			}
			if e := restarted.Reconcile(); e != nil {
				t.Fatal("original context cannot be inspected after bounded cleanup", e)
			}
		})
	}
}

func TestLifetimeWatchdogCannotFollowPollingOwnerLifetime(t *testing.T) {
	ended := make(chan struct{})
	deadline := make(chan time.Time)
	stopped := make(chan struct{})
	go watchLifetime(ended, deadline, func() { close(stopped) })
	// Polling owner restart does not close the helper's private pipe.
	select {
	case <-stopped:
		t.Fatal("owner restart killed command")
	default:
	}
	close(ended)
	awaitFake(t, stopped)
}

func TestLifetimeNativeEntryGuardsAndUnixWiring(t *testing.T) {
	if allowNativeExecution {
		t.Fatal("native guard open")
	}
	if e := superviseHelper(time.Now().Unix() + 30); e == nil || !strings.Contains(e.Error(), "ISOLATION_GUARD") {
		t.Fatal("native supervision guard missing", e)
	}
	// Unix-only entry has the same early guard; Windows rejects this mode.
	if e := watchdogMain(0, 0); e == nil {
		t.Fatal("native watchdog entered")
	}
	src, e := os.ReadFile("os_unix.go")
	if e != nil {
		t.Fatal(e)
	}
	for _, part := range []string{"Pgid: os.Getpid()", "startHelper(c)", "io.ReadFull(readyRead", "helperLifetime = lifeWrite", "syscall.Kill(-syscall.Getpgrp()", "watchLifetime(ended, timer.C", "os.Getppid() != group"} {
		if !strings.Contains(string(src), part) {
			t.Fatal("missing Unix adapter invariant", part)
		}
	}
	t.Log("STATIC_ONLY Unix private-pipe/group pin/readiness wiring; injected lifecycle behavior only; native NOT_RUN")
}
