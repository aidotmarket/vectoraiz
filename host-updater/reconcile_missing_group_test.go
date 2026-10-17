package main

import (
	"errors"
	"path/filepath"
	"reflect"
	"testing"
	"time"
)

// All observations, locks and time are injected; no native child is started.
func TestR386PredeadlineMissingGroup(t *testing.T) {
	for _, kind := range []string{"absent", "absent-run", "becomes-absent", "live", "held-lock", "done-race", "live-race", "identity-race", "not-started-race", "unknown-identity", "negative-identity", "not-started"} {
		t.Run(kind, func(t *testing.T) {
			if allowNativeExecution {
				t.Fatal("native execution guard open")
			}
			root := t.TempDir()
			path := filepath.Join(root, "child.json")
			now := int64(10000)
			began := now
			r := Command{ID: "original", Started: true, ProcessGroup: 43210, Deadline: now + 1800, Args: []string{"never-execute"}, Env: []string{"SECRET=fake"}, Input: []byte("fake"), Max: 32}
			switch kind {
			case "unknown-identity":
				r.ProcessGroup = 0
			case "negative-identity":
				r.ProcessGroup = -1
			case "not-started":
				r.Started = false
			}
			write := func() {
				t.Helper()
				if e := save(path, &r); e != nil {
					t.Fatal(e)
				}
			}
			write()
			locks := newMemoryLocks()
			child := filepath.Join(root, "child.lock")
			if kind == "held-lock" {
				release, e := locks.acquire(child)
				if e != nil {
					t.Fatal(e)
				}
				defer release()
			}
			observations, acquisitions := 0, 0
			p := Processes{Root: root, Now: func() time.Time { return time.Unix(now, 0) }, Pause: func() {
				now++
				// Unresolved controls stop before the deadline. A durable helper result
				// lets them finish without accidentally testing postdeadline recovery.
				if now == began+3 && kind != "absent" && kind != "absent-run" && kind != "becomes-absent" {
					r.Done, r.Code, r.Output = true, "command_exit", []byte("durable")
					r.Args, r.Env, r.Input = nil, nil, nil
					write()
					locks.mu.Lock()
					delete(locks.held, child)
					locks.mu.Unlock()
				}
			}, GroupAlive: func(pid int) bool {
				observations++
				if !r.Started || pid <= 0 || pid != r.ProcessGroup {
					t.Fatal("observed unidentified or not-started group", pid)
				}
				return kind == "live" || kind == "live-race" && acquisitions > 0 || kind == "identity-race" && acquisitions > 0 || kind == "becomes-absent" && now == began
			}}
			p.Acquire = func(name string) (func(), error) {
				release, e := locks.acquire(name)
				if e != nil || name != child {
					return release, e
				}
				acquisitions++
				if acquisitions == 1 {
					switch kind {
					case "done-race":
						r.Done, r.Code, r.Output = true, "command_exit", []byte("durable")
						r.Args, r.Env, r.Input = nil, nil, nil
						write()
					case "identity-race":
						r.ProcessGroup++
						write()
					case "not-started-race":
						r.Started = false
						write()
					}
				}
				return func() {
					var got Command
					if e := load(path, &got); e != nil {
						t.Fatal(e)
					}
					if (kind == "absent" || kind == "absent-run" || kind == "becomes-absent" || kind == "done-race") && !got.Done {
						t.Fatal("child lock released before durable Done")
					}
					locks.mu.Lock()
					handoffHeld := locks.held[filepath.Join(root, "handoff.lock")]
					locks.mu.Unlock()
					if !handoffHeld {
						t.Fatal("handoff serialization lost")
					}
					release()
				}, nil
			}
			p.StartCommand = func(Command) error { t.Fatal("replayed command"); return errors.New("unexpected_start") }
			var err error
			if kind == "absent-run" {
				_, err = p.Run([]string{"must-not-launch"}, nil, r.Deadline, 32, true)
			} else {
				err = p.Reconcile()
			}
			var got Command
			if e := load(path, &got); e != nil {
				t.Fatal(e)
			}
			if kind == "absent" || kind == "absent-run" || kind == "becomes-absent" {
				want := r
				want.Done, want.Code = true, "child_outcome_unknown"
				want.Args, want.Env, want.Input = nil, nil, nil
				if err == nil || err.Error() != "child_outcome_unknown" || !reflect.DeepEqual(got, want) || now-began > 1 {
					t.Fatalf("missing group: elapsed=%ds observations=%d result=%v record=%+v", now-began, observations, err, got)
				}
				t.Logf("sealed unknown after %ds fake time; observations=%d; original deadline=%d", now-began, observations, got.Deadline)
			} else if err != nil || !reflect.DeepEqual(got, r) || got.Code != "command_exit" {
				t.Fatalf("durable completion overwritten or control sealed early: elapsed=%ds result=%v record=%+v", now-began, err, got)
			}
			if (kind == "unknown-identity" || kind == "negative-identity" || kind == "not-started") && observations != 0 {
				t.Fatal("unsafe group observation", observations)
			}
			if kind == "done-race" && now != began {
				t.Fatal("did not reread Done promptly under acquired lock")
			}
			if e := p.Reconcile(); e != nil {
				t.Fatal("durable result not reusable", e)
			}
			locks.mu.Lock()
			defer locks.mu.Unlock()
			if len(locks.held) != 0 {
				t.Fatal("ownership leaked", locks.held)
			}
		})
	}
}
