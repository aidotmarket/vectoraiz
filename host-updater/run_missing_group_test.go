package main

import (
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"testing"
	"time"
)

// Exercise Run's own polling after exactly one fresh fake launch. No native
// adapters, subprocesses, signals, or wall-clock deadline waits are used.
func TestR386RunMissingHelperAfterLaunch(t *testing.T) {
	for _, kind := range []string{"absent", "becomes-absent", "live", "held-lock", "done-race", "success-race", "live-race", "group-race", "not-started-race", "id-race", "done-id-race", "poll-id-race", "unknown-identity", "negative-identity", "not-started", "lock-error", "load-error", "save-error", "postdeadline"} {
		t.Run(kind, func(t *testing.T) {
			if allowNativeExecution {
				t.Fatal("native execution guard open")
			}
			root := t.TempDir()
			path := filepath.Join(root, "child.json")
			child := filepath.Join(root, "child.lock")
			handoff := filepath.Join(root, "handoff.lock")
			locks := newMemoryLocks()
			now, began := int64(10000), int64(10000)
			starts, observations, acquisitions := 0, 0, 0
			var record Command
			write := func() {
				t.Helper()
				if e := save(path, &record); e != nil {
					t.Fatal(e)
				}
			}
			held := func(name string) bool { locks.mu.Lock(); defer locks.mu.Unlock(); return locks.held[name] }
			complete := func() {
				record.Done, record.Code, record.Output = true, "command_exit", []byte("durable")
				record.Args, record.Env, record.Input = nil, nil, nil
				write()
			}
			injected := errors.New("fake_lock_failure")
			p := Processes{Root: root, Env: []string{"SECRET=fake"}, Cwd: "original-cwd", Now: func() time.Time { return time.Unix(now, 0) }}
			p.StartCommand = func(r Command) error {
				starts++
				if starts != 1 || !held(handoff) {
					t.Fatal("launch serialization/replay")
				}
				record = r
				record.Started, record.ProcessGroup = true, 43210
				switch kind {
				case "unknown-identity":
					record.ProcessGroup = 0
				case "negative-identity":
					record.ProcessGroup = -1
				case "not-started":
					record.Started = false
				case "postdeadline":
					now = r.Deadline + 6
				case "poll-id-race":
					record.ID = "replacement"
				}
				write()
				return nil
			}
			p.GroupAlive = func(pid int) bool {
				observations++
				if !record.Started || pid <= 0 || pid != record.ProcessGroup {
					t.Fatal("unsafe group observation", pid)
				}
				return kind == "live" || kind == "becomes-absent" && now == began || (kind == "live-race" || kind == "group-race") && acquisitions > 0
			}
			p.Pause = func() {
				now++
				if now == began+3 {
					complete()
				}
			}
			p.Acquire = func(name string) (func(), error) {
				if name == child && !record.Done {
					if kind == "held-lock" {
						return nil, errors.New("installation_busy")
					}
					if kind == "lock-error" {
						return nil, injected
					}
				}
				release, e := locks.acquire(name)
				if e != nil {
					return nil, e
				}
				if name == child {
					acquisitions++
					if acquisitions == 1 {
						switch kind {
						case "done-race", "success-race":
							complete()
							if kind == "success-race" {
								record.Code = ""
								write()
							}
						case "live-race":
						case "group-race":
							record.ProcessGroup++
							write()
						case "not-started-race":
							record.Started = false
							write()
						case "id-race", "done-id-race":
							record.ID = "replacement"
							if kind == "done-id-race" {
								complete()
							} else {
								write()
							}
						case "load-error":
							if e := os.Remove(path); e != nil {
								t.Fatal(e)
							}
						}
					}
				}
				return func() {
					if !held(handoff) {
						t.Fatal("handoff released before child/result collection")
					}
					var got Command
					if e := load(path, &got); e == nil && name == child && got.Code == "child_outcome_unknown" {
						if !held(child) || !got.Done || got.Args != nil || got.Env != nil || got.Input != nil {
							t.Fatal("unknown not durably scrubbed under ownership")
						}
					}
					release()
				}, nil
			}
			// Make atomic rename fail only after the under-lock liveness recheck.
			originalAlive := p.GroupAlive
			p.GroupAlive = func(pid int) bool {
				alive := originalAlive(pid)
				if kind == "save-error" && acquisitions > 0 {
					if e := os.Remove(path); e != nil {
						t.Fatal(e)
					}
					if e := os.Mkdir(path, 0700); e != nil {
						t.Fatal(e)
					}
				}
				return alive
			}
			out, err := p.Run([]string{"never-execute", "fake-secret"}, []byte("fake-input"), began+1800, 32, true)
			if starts != 1 || len(locks.held) != 0 {
				t.Fatal("launch count/ownership leaked", starts, locks.held)
			}
			switch kind {
			case "absent", "becomes-absent":
				var got Command
				if e := load(path, &got); e != nil {
					t.Fatal(e)
				}
				want := record
				want.Done, want.Code = true, "child_outcome_unknown"
				want.Args, want.Env, want.Input = nil, nil, nil
				if err == nil || err.Error() != want.Code || !reflect.DeepEqual(got, want) || now-began > 1 || observations < 2 {
					t.Fatalf("elapsed=%ds observations=%d err=%v record=%+v", now-began, observations, err, got)
				}
				t.Logf("fresh launches=%d; elapsed=%ds; observations=%d; deadline=%d; sealed and scrubbed under locks", starts, now-began, observations, got.Deadline)
			case "id-race", "done-id-race", "poll-id-race":
				var got Command
				if e := load(path, &got); e != nil {
					t.Fatal(e)
				}
				if err == nil || err.Error() != "child_identity" || !reflect.DeepEqual(got, record) || now != began {
					t.Fatal("replacement accepted/overwritten", err, got)
				}
			case "lock-error":
				if !errors.Is(err, injected) || now != began {
					t.Fatal("lock error lost", err)
				}
			case "load-error":
				if !os.IsNotExist(err) || now != began {
					t.Fatal("load error lost", err)
				}
			case "save-error":
				var pe *os.LinkError
				if !errors.As(err, &pe) || now != began {
					t.Fatal("save error lost", err)
				}
			case "postdeadline":
				if err == nil || err.Error() != "child_may_be_running" || observations != 0 || acquisitions != 0 {
					t.Fatal("postdeadline behavior changed", err, observations, acquisitions)
				}
			default:
				wantCode := "command_exit"
				if kind == "success-race" {
					wantCode = ""
				}
				if (err == nil) != (wantCode == "") || err != nil && err.Error() != wantCode || string(out) != "durable" {
					t.Fatal("durable outcome changed", err, string(out))
				}
				if (kind == "done-race" || kind == "success-race") && now != began {
					t.Fatal("racing completion delayed")
				}
				if kind != "done-race" && kind != "success-race" && now != began+3 {
					t.Fatal("control sealed prematurely", now-began)
				}
				if (kind == "unknown-identity" || kind == "negative-identity" || kind == "not-started") && observations != 0 {
					t.Fatal("unidentified observation")
				}
				var got Command
				if e := load(path, &got); e != nil || !reflect.DeepEqual(got, record) {
					t.Fatal("control record changed", e, got)
				}
			}
		})
	}
}
