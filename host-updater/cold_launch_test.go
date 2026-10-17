package main

import (
	"errors"
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
						deadline = *now + o.S.Limits.Resolve
						generation++
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
					if generation > 1 && (len(o.S.History) != 1 || o.S.History[0].ID != oldID) {
						t.Fatal("retry lost failed adoption history")
					}
					if expires {
						assertPending()
						if e == nil || e.Error() != "daemon_wait_deadline" || waits != 0 || *now != deadline {
							t.Fatal("outage not bounded/truthful", e, waits, *now)
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
