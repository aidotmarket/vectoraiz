package main

import (
	"errors"
	"os"
	"path/filepath"
	"testing"
	"time"
)

// Exercise the shared ordinary-launch branch, including automatic retry and
// the supervisor handoff. All Docker, desktop startup and time are fakes.
func TestColdPendingLaunch(t *testing.T) {
	if allowNativeExecution {
		t.Fatal("native execution guard open")
	}
	for _, phase := range []string{"failed_resolution", "failed_download", "accepted", "verifying"} {
		for _, runtime := range []struct{ platform, host, name string }{
			{"darwin", "unix:///var/run/docker.sock", "Docker"},
			{"darwin", "unix:///Users/person/.orbstack/run/docker.sock", "OrbStack"},
			{"windows", "npipe:////./pipe/docker_engine", "Docker Desktop"},
			{"darwin", "ssh://already-authorized-B", ""},
		} {
			for _, expires := range []bool{false, true} {
				outcome := "returns"
				if expires {
					outcome = "expires"
				}
				t.Run(phase+"/"+runtime.name+"/"+outcome, func(t *testing.T) {
					o, f, now := fixture(t)
					switch phase {
					case "failed_resolution", "failed_download":
						f.pullFail = true
						submitTest(t, o)
						drive(t, o, now)
						// Both confirmed preactivation failures take the same retry path.
						o.S.Current.Phase = phase
						if e := o.persist(); e != nil {
							t.Fatal(e)
						}
						f.pullFail = false
					default:
						reachWorkflowPhase(t, o, now, phase)
					}
					oldID, generation := o.S.Current.ID, o.S.Generation
					deadline := o.S.Current.Deadline
					if phase == "failed_resolution" || phase == "failed_download" {
						// Failed work stays unchanged until startup succeeds.
						if o.S.Desired != nil || o.S.LastVerified != nil {
							t.Fatal("retry must exercise failed first adoption")
						}
					}
					endpoint := jsonBytes([]any{map[string]any{"Endpoints": map[string]any{"docker": map[string]string{"Host": runtime.host}}, "TLSMaterial": nil}})
					o.D.Rec.Endpoint = endpoint
					o.D.R = workflowRunner{f, func(a []string, b []byte, d int64, m int, v bool) ([]byte, error, bool) {
						if a[3] == "context" {
							return endpoint, nil, true
						}
						return nil, nil, false
					}}
					f.daemonDown = true
					up, began := f.up, *now
					starts, waits := 0, 0
					assertPending := func() {
						t.Helper()
						if o.S.Current.Deadline != deadline || o.S.Generation != generation || f.up != up {
							t.Fatal("availability renewed deadline or replayed activation", o.S.Current)
						}
					}
					pending, e := o.launchPending(func() time.Time { return time.Unix(*now, 0) }, func() {
						assertPending()
						*now++
						if !expires && *now == began+3 {
							f.daemonDown = false
						}
					}, func() error {
						// Same selection used by startRuntime, with the platform supplied
						// explicitly so these tests never invoke native desktop programs.
						if got := o.D.desktopRuntimeFor(runtime.platform); got != runtime.name {
							t.Fatal("wrong runtime", got)
						}
						if runtime.name != "" {
							starts++
						}
						return nil
					}, func() error {
						waits++
						if phase == "failed_resolution" || phase == "failed_download" {
							deadline = *now + o.S.Limits.Resolve
							generation++
						}
						assertPending()
						if f.daemonDown {
							t.Fatal("wait reached before runtime available")
						}
						// Model the existing release/wait handoff by reloading durable
						// state as the supervisor does, then continue the actual controller.
						restarted, re := openOwner(o.Root, o.D)
						if re != nil {
							return re
						}
						restarted.Now = o.Now
						o = restarted
						drive(t, o, now)
						if o.S.Current.Phase != "succeeded" || f.up != 1 {
							t.Fatal("cold launch did not continue exactly once", o.S.Current, f.up)
						}
						return nil
					})
					wantStarts := 1
					if runtime.name == "" {
						wantStarts = 0
					}
					if !pending || starts != wantStarts || o.S.Current.Target != "v2.0.0" {
						t.Fatal("wrong shared launch branch/start/target", pending, starts, e)
					}
					if !expires && (phase == "failed_resolution" || phase == "failed_download") && (len(o.S.History) != 1 || o.S.History[0].ID != oldID) {
						t.Fatal("retry lost failed adoption history")
					}
					if expires {
						assertPending()
						until := deadline
						if phase == "failed_resolution" || phase == "failed_download" {
							until = began + o.S.Limits.Activate
						}
						if e == nil || e.Error() != "daemon_wait_deadline" || waits != 0 || *now != until {
							t.Fatal("outage not bounded/truthful", e, waits, *now)
						}
						if phase == "failed_resolution" || phase == "failed_download" {
							if o.S.Current.ID != oldID || len(o.S.History) != 0 {
								t.Fatal("failed startup resubmitted retry")
							}
							return
						}
						code := "phase_deadline"
						if phase == "verifying" {
							code = "startup_failed_migration_unconfirmed"
						}
						if e = o.step(); e != nil || o.S.Current.active() || o.S.Current.Code != code {
							t.Fatal("supervisor lost terminal deadline", e, o.S.Current)
						}
					} else if e != nil || waits != 1 {
						t.Fatal("successful availability did not hand off once", e, waits)
					}
				})
			}
		}
	}
}

func TestColdPendingLaunchRefusesDriftAndUnknown(t *testing.T) {
	for _, kind := range []string{"source", "context", "recovery_required"} {
		t.Run(kind, func(t *testing.T) {
			o, f, now := fixture(t)
			reachWorkflowPhase(t, o, now, "verifying")
			deadline, up := o.S.Current.Deadline, f.up
			switch kind {
			case "source":
				f.drift = true
			case "context":
				o.D.Rec.Endpoint = jsonBytes([]any{})
			case "recovery_required":
				o.S.Current.Phase = "recovery_required"
			}
			pending, e := o.launchPending(func() time.Time { return time.Unix(*now, 0) }, func() {
				t.Fatal("drift waited")
			}, func() error {
				t.Fatal("drift started runtime")
				return nil
			}, func() error {
				t.Fatal("drift entered wait")
				return errors.New("unexpected_wait")
			})
			if kind == "recovery_required" {
				if pending || e != nil {
					t.Fatal("unknown activation resumed", e)
				}
			} else if !pending || e == nil {
				t.Fatal("drift accepted", e)
			}
			if f.up != up || o.S.Current.Deadline != deadline {
				t.Fatal("refusal replayed activation or changed deadline")
			}
		})
	}
}

func TestR384PastDeadlineFirstClick(t *testing.T) {
	if allowNativeExecution {
		t.Fatal("native guard open")
	}
	for _, phase := range []string{"accepted", "resolving", "downloading"} {
		t.Run(phase, func(t *testing.T) {
			o, f, now := fixture(t)
			reachWorkflowPhase(t, o, now, phase)
			old := o.S.Current
			*now = old.Deadline + 1
			waits := 0
			pending, err := o.launchPending(func() time.Time { return time.Unix(*now, 0) }, func() { *now++ }, func() error { t.Fatal("available runtime started"); return nil }, func() error {
				waits++
				if o.S.Current.ID == old.ID || o.S.Current.Target != old.Target || len(o.S.History) != 1 || o.S.History[0] != old || old.Code != "phase_deadline" || o.S.Current.Deadline != *now+o.S.Limits.Resolve {
					t.Fatal("expiry retry lost target/history or resolution budget", o.S.Current)
				}
				drive(t, o, now)
				return nil
			})
			if !pending || err != nil || waits != 1 || f.up != 1 || o.S.Current.Phase != "succeeded" {
				t.Fatalf("FIRST_CLICK_NOT_RECOVERED: pending=%v error=%v waits=%d activation=%d phase=%s", pending, err, waits, f.up, o.S.Current.Phase)
			}
		})
	}
}

func TestR384RetryStartupHasItsOwnBudget(t *testing.T) {
	if allowNativeExecution {
		t.Fatal("native guard open")
	}
	o, f, now := fixture(t)
	f.pullFail = true
	submitTest(t, o)
	drive(t, o, now)
	f.pullFail = false
	if o.S.Current.Phase != "failed_download" {
		t.Fatal("wrong setup", o.S.Current.Phase)
	}
	began := *now
	f.daemonDown = true
	waits := 0
	pending, err := o.launchPending(func() time.Time { return time.Unix(*now, 0) }, func() {
		*now++
		if *now-began == 11 {
			f.daemonDown = false
		}
	}, func() error { return nil }, func() error { waits++; drive(t, o, now); return nil })
	if !pending || err != nil || waits != 1 || f.up != 1 || o.S.Current.Phase != "succeeded" {
		t.Fatalf("RETRY_STARTUP_CONSUMED_RESOLUTION: elapsed=%d pending=%v error=%v waits=%d activation=%d phase=%s", *now-began, pending, err, waits, f.up, o.S.Current.Phase)
	}
}

func TestR384PendingWaitPublishesFreshStatus(t *testing.T) {
	if allowNativeExecution {
		t.Fatal("native guard open")
	}
	o, f, now := fixture(t)
	o.S.Limits.Resolve = 40
	submitTest(t, o)
	if err := o.hostStatus(); err != nil {
		t.Fatal(err)
	}
	began := *now
	f.daemonDown = true
	_, err := o.launchPending(func() time.Time { return time.Unix(*now, 0) }, func() {
		*now++
		var status Snapshot
		if e := load(filepath.Join(o.Root, "status.json"), &status); e != nil {
			t.Fatal(e)
		}
		if *now-status.Heartbeat > 5 || status.Reason != "launch_waiting" || status.Ready {
			t.Errorf("STALE_PENDING_HEARTBEAT: age=%d reason=%q ready=%v", *now-status.Heartbeat, status.Reason, status.Ready)
		}
		if *now-began == 20 {
			f.daemonDown = false
		}
	}, func() error { return nil }, func() error { drive(t, o, now); return nil })
	if err != nil {
		t.Fatal(err)
	}
}

func TestR384UnexpiredAvailableControl(t *testing.T) {
	if allowNativeExecution {
		t.Fatal("native guard open")
	}
	o, f, now := fixture(t)
	submitTest(t, o)
	pending, err := o.launchPending(func() time.Time { return time.Unix(*now, 0) }, func() { *now++ }, func() error { return nil }, func() error { drive(t, o, now); return nil })
	if !pending || err != nil || f.up != 1 || o.S.Current.Phase != "succeeded" {
		t.Fatal("positive control", pending, err, f.up, o.S.Current.Phase)
	}
}

// Expiry reconciliation must never turn post-activation or uncertain work into
// a new accepted operation, even when the daemon is available on the first click.
func TestR384ExpiredRecoveryCannotRetry(t *testing.T) {
	for _, phase := range []string{"downloaded", "activating", "verifying", "accepted-issued"} {
		t.Run(phase, func(t *testing.T) {
			o, f, now := fixture(t)
			setup := phase
			if phase == "accepted-issued" {
				setup = "accepted"
			}
			reachWorkflowPhase(t, o, now, setup)
			o.S.Current.ActivationIssued = true
			old, up := o.S.Current, f.up
			deadline, generation := old.Deadline, o.S.Generation
			*now = deadline
			pending, err := o.launchPending(func() time.Time { return time.Unix(*now, 0) }, func() { t.Fatal("recovery waited") }, func() error { t.Fatal("recovery started runtime"); return nil }, func() error { t.Fatal("recovery retried"); return nil })
			if phase == "accepted-issued" {
				if !pending || err == nil || err.Error() != "confirmed_pre_activation_failure_required" {
					t.Fatal(pending, err)
				}
			} else if pending || err != nil || old.Phase != "recovery_required" {
				t.Fatal(pending, err, old)
			}
			if o.S.Current != old || old.Deadline != deadline || o.S.Generation != generation || len(o.S.History) != 0 || f.up != up {
				t.Fatal("uncertain activation renewed or replayed")
			}
		})
	}
}

func TestR384PendingStatusFinalizedBeforeHandoff(t *testing.T) {
	for _, outcome := range []string{"success", "deadline", "startup-unknown", "wait-error"} {
		t.Run(outcome, func(t *testing.T) {
			o, f, now := fixture(t)
			submitTest(t, o)
			if outcome == "startup-unknown" {
				o.S.Current.Phase = "failed_resolution"
			}
			f.daemonDown = outcome == "deadline" || outcome == "startup-unknown"
			_, err := o.launchPending(func() time.Time { return time.Unix(*now, 0) }, func() { *now++ }, func() error {
				if outcome == "startup-unknown" {
					return errors.New("child_outcome_unknown")
				}
				return nil
			}, func() error {
				var status Snapshot
				if e := load(filepath.Join(o.Root, "status.json"), &status); e != nil || status.Reason != "" {
					t.Fatal("waiting status leaked into handoff", e, status.Reason)
				}
				// Model a newer status published after the callback releases the lock.
				o.Reason = "newer_owner_status"
				if e := o.hostStatus(); e != nil {
					t.Fatal(e)
				}
				if outcome == "wait-error" {
					return errors.New("supervisor_wait_failed")
				}
				return nil
			})
			var status Snapshot
			if e := load(filepath.Join(o.Root, "status.json"), &status); e != nil {
				t.Fatal(e)
			}
			want := "newer_owner_status"
			if outcome == "deadline" {
				want = "daemon_wait_deadline"
			}
			if outcome == "startup-unknown" {
				want = "child_outcome_unknown"
			}
			if status.Reason != want || status.Heartbeat != *now {
				t.Fatal("untruthful final status", status.Reason, status.Heartbeat)
			}
			if (err == nil) != (outcome == "success") {
				t.Fatal("outcome hidden", err)
			}
		})
	}
}

func TestR384RetryRefusesDriftBeforeResubmission(t *testing.T) {
	for _, kind := range []string{"source", "context", "credential"} {
		t.Run(kind, func(t *testing.T) {
			o, f, now := fixture(t)
			f.pullFail = true
			submitTest(t, o)
			drive(t, o, now)
			old, generation := o.S.Current, o.S.Generation
			switch kind {
			case "source":
				f.drift = true
			case "context":
				o.D.Rec.Endpoint = jsonBytes([]any{})
			case "credential":
				if e := os.MkdirAll(filepath.Join(o.Root, ".docker"), 0700); e != nil {
					t.Fatal(e)
				}
				if e := os.WriteFile(filepath.Join(o.Root, ".docker", "config.json"), []byte(`{"auths":{"registry.example":{}}}`), 0600); e != nil {
					t.Fatal(e)
				}
			}
			_, err := o.launchPending(func() time.Time { return time.Unix(*now, 0) }, func() { t.Fatal("drift waited") }, func() error { t.Fatal("drift started runtime"); return nil }, func() error { t.Fatal("drift submitted"); return nil })
			if err == nil || o.S.Current != old || o.S.Generation != generation || len(o.S.History) != 0 || f.up != 0 {
				t.Fatal("drift authorized retry", err)
			}
		})
	}
}
