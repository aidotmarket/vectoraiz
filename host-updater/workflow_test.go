package main

import (
	"bytes"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

type workflowRunner struct {
	inner     Runner
	intercept func([]string, []byte, int64, int, bool) ([]byte, error, bool)
}

func (r workflowRunner) Run(a []string, b []byte, d int64, m int, v bool) ([]byte, error) {
	if out, e, ok := r.intercept(a, b, d, m, v); ok {
		return out, e
	}
	return r.inner.Run(a, b, d, m, v)
}
func threeMounts() []map[string]any {
	return []map[string]any{
		{"Type": "volume", "Source": "/vol/data", "Destination": "/data", "RW": true, "Name": "data", "Driver": "local", "Mode": "rw", "Propagation": ""},
		{"Type": "bind", "Source": "/imports", "Destination": "/imports", "RW": true, "Mode": "rw", "Propagation": "rprivate"},
		{"Type": "bind", "Source": "/var/run/docker.sock", "Destination": "/var/run/docker.sock", "RW": false, "Mode": "ro", "Propagation": "rprivate"},
	}
}
func TestWorkflowMountMultiset(t *testing.T) {
	a := ownedIdentity("app", oldImage, "vectoraiz")
	a.Mounts = threeMounts()
	for _, p := range [][]int{{0, 1, 2}, {0, 2, 1}, {1, 0, 2}, {1, 2, 0}, {2, 0, 1}, {2, 1, 0}} {
		b := a
		b.Mounts = []map[string]any{a.Mounts[p[0]], a.Mounts[p[1]], a.Mounts[p[2]]}
		if !sameRuntime(a, b) {
			t.Fatal("incidental order changed identity", p)
		}
	}
	for _, field := range []string{"Type", "Source", "Destination", "RW", "Name", "Driver", "Mode", "Propagation", "Options"} {
		b := a
		b.Mounts = nil
		json.Unmarshal(jsonBytes(a.Mounts), &b.Mounts)
		b.Mounts[0][field] = "changed"
		if sameRuntime(a, b) {
			t.Fatal("discarded mount field", field)
		}
	}
	for _, mounts := range [][]map[string]any{a.Mounts[:2], append(append([]map[string]any{}, a.Mounts...), a.Mounts[0]), {a.Mounts[0], a.Mounts[0], a.Mounts[2]}} {
		b := a
		b.Mounts = mounts
		if sameRuntime(a, b) {
			t.Fatal("lost missing/extra/duplicate mount")
		}
	}
}
func TestWorkflowShuffledMountLifecycle(t *testing.T) {
	o, f, now := fixture(t)
	f.app.Mounts = threeMounts()
	base := o.D.Rec.Containers["vectoraiz"]
	base.Mounts = threeMounts()
	o.D.Rec.Containers["vectoraiz"] = base
	count := 0
	o.D.R = workflowRunner{f, func(a []string, b []byte, d int64, m int, v bool) ([]byte, error, bool) {
		if len(a) > 3 && a[3] == "inspect" {
			out, e := f.Run(a, b, d, m, v)
			if e != nil {
				return out, e, true
			}
			var ids []Identity
			json.Unmarshal(out, &ids)
			mounts := ids[0].Mounts
			if len(mounts) == 3 {
				count++
				k := count % 3
				ids[0].Mounts = []map[string]any{mounts[k], mounts[(k+1)%3], mounts[(k+2)%3]}
			}
			return jsonBytes(ids), nil, true
		}
		return nil, nil, false
	}}
	submitTest(t, o)
	drive(t, o, now)
	if o.S.Current.Phase != "succeeded" || f.up != 1 {
		t.Fatalf("shuffle failed %+v", o.S.Current)
	}
}
func reachWorkflowPhase(t *testing.T, o *Owner, now *int64, phase string) {
	t.Helper()
	submitTest(t, o)
	for i := 0; i < 20 && o.S.Current.Phase != phase; i++ {
		if e := o.step(); e != nil {
			t.Fatal(e)
		}
		*now++
	}
	if o.S.Current.Phase != phase {
		t.Fatalf("did not reach %s: %+v", phase, o.S.Current)
	}
}
func TestWorkflowOutageEachPhaseAcrossRestart(t *testing.T) {
	for _, phase := range []string{"accepted", "resolving", "downloading", "downloaded", "activating", "verifying"} {
		for _, expired := range []bool{false, true} {
			t.Run(phase+map[bool]string{false: "/returns", true: "/expires"}[expired], func(t *testing.T) {
				o, f, now := fixture(t)
				reachWorkflowPhase(t, o, now, phase)
				deadline := o.S.Current.Deadline
				up := f.up
				f.daemonDown = true
				for n := 0; n < 2; n++ {
					restarted, e := openOwner(o.Root, o.D)
					if e != nil {
						t.Fatal(e)
					}
					restarted.Now = o.Now
					o = restarted
					if e = o.step(); e != nil {
						t.Fatal(e)
					}
					if o.S.Current.Phase != phase || o.S.Current.Deadline != deadline || f.up != up {
						t.Fatalf("outage changed operation %+v", o.S.Current)
					}
					*now++
				}
				if expired {
					*now = deadline
					if e := o.step(); e != nil {
						t.Fatal(e)
					}
					if o.S.Current.active() || o.S.Current.Code != "phase_deadline" {
						t.Fatalf("deadline hidden %+v", o.S.Current)
					}
					return
				}
				f.daemonDown = false
				drive(t, o, now)
				if o.S.Current.Phase != "succeeded" || f.up != 1 {
					t.Fatalf("did not resume %+v up=%d", o.S.Current, f.up)
				}
			})
		}
	}
}
func TestWorkflowOutageDoesNotHideDriftOrUnknown(t *testing.T) {
	for _, kind := range []string{"source", "dependency", "handoff"} {
		t.Run(kind, func(t *testing.T) {
			o, f, now := fixture(t)
			reachWorkflowPhase(t, o, now, "activating")
			switch kind {
			case "source":
				f.drift = true
			case "dependency":
				f.dep.Image = targetImage
			case "handoff":
				o.D.R = workflowRunner{f, func(a []string, b []byte, d int64, m int, v bool) ([]byte, error, bool) {
					if strings.Contains(strings.Join(a, " "), "config --format") {
						return nil, errors.New("child_outcome_unknown"), true
					}
					return nil, nil, false
				}}
			}
			if e := o.step(); e != nil {
				t.Fatal(e)
			}
			if o.S.Current.Phase != "recovery_required" || f.up != 0 {
				t.Fatalf("drift/unknown masked %+v", o.S.Current)
			}
		})
	}
}
func TestWorkflowColdLaunchAndBoundedUnavailable(t *testing.T) {
	for _, expired := range []bool{false, true} {
		t.Run(map[bool]string{false: "returns", true: "expires"}[expired], func(t *testing.T) {
			o, f, now := fixture(t)
			submitTest(t, o)
			drive(t, o, now)
			f.daemonDown = true
			start := *now
			starts := 0
			clock := func() time.Time { return time.Unix(*now, 0) }
			pause := func() {
				*now++
				if !expired && *now-start == 3 {
					f.daemonDown = false
				}
			}
			e := o.launchSelected(clock, pause, func() error { starts++; return nil })
			if expired {
				if e == nil || e.Error() != "daemon_wait_deadline" || *now != start+o.S.Limits.Activate {
					t.Fatal("unbounded/untruthful outage", e, *now-start)
				}
			} else if e != nil {
				t.Fatal(e)
			}
			if starts != 1 || f.up != 1 {
				t.Fatal("start or activation duplicated", starts, f.up)
			}
			if o.D.desktopRuntime() != "" {
				t.Fatal("remote context started a desktop")
			}
		})
	}
}
func TestWorkflowRetryKeepsTargetAndUnknownRefuses(t *testing.T) {
	o, f, now := fixture(t)
	f.pullFail = true
	submitTest(t, o)
	drive(t, o, now)
	old := o.S.Current.ID
	if e := o.resumeSelection(); e != nil {
		t.Fatal(e)
	}
	if o.S.Current.Target != "v2.0.0" || o.S.Current.ID == old || len(o.S.History) != 1 {
		t.Fatal("retry lost exact target/history")
	}
	f.pullFail = false
	drive(t, o, now)
	if o.S.Current.Phase != "succeeded" || f.up != 1 {
		t.Fatal("adoption retry failed")
	}
	o.S.Current.Phase = "recovery_required"
	o.S.Current.ActivationIssued = true
	before := f.up
	if e := o.resumeSelection(); e == nil || f.up != before {
		t.Fatal("unknown activation replayed")
	}
}
func TestWorkflowRefreshKeepsSelectedImage(t *testing.T) {
	o, f, now := fixture(t)
	submitTest(t, o)
	drive(t, o, now)
	selected := *o.S.Desired
	installation := o.D.Rec.Installation
	pair := o.D.Rec.Pair
	// A legitimate app-only configuration change and a new credential source.
	var config map[string]any
	json.Unmarshal(f.config, &config)
	config["services"].(map[string]any)["vectoraiz"].(map[string]any)["environment"].(map[string]any)["PASSWORD"] = "new-private-value"
	f.config = jsonBytes(config)
	os.MkdirAll(filepath.Join(o.Root, ".docker"), 0700)
	os.WriteFile(filepath.Join(o.Root, ".docker", "config.json"), []byte(`{"auths":{"registry.example":{}}}`), 0600)
	if e := o.D.revalidate(); e == nil {
		t.Fatal("silently trusted changed context/config")
	}
	if e := o.refresh(""); e != nil {
		t.Fatal(e)
	}
	if *o.S.Desired != selected || *o.S.LastVerified != selected || o.D.Rec.Installation != installation || o.D.Rec.Pair != pair {
		t.Fatal("refresh replaced selection/identity")
	}
	if e := o.D.revalidate(); e != nil {
		t.Fatal(e)
	}
	e := o.launchSelected(func() time.Time { return time.Unix(*now, 0) }, func() { *now++ }, func() error { t.Fatal("started remote desktop"); return nil })
	if e == nil || e.Error() != "launch_verification_pending" {
		t.Fatal("changed configuration did not activate selected image", e)
	}
	drive(t, o, now)
	if o.S.Current.Phase != "succeeded" || f.app.Image != selected.ImageID || f.up != 2 {
		t.Fatalf("refresh launch failed %+v", o.S.Current)
	}
}
func TestWorkflowRefreshRefusesUnownedContextAndPending(t *testing.T) {
	for _, kind := range []string{"other-container", "other-image", "mount", "pending", "unknown"} {
		t.Run(kind, func(t *testing.T) {
			o, f, _ := fixture(t)
			old := jsonBytes(o.D.Rec)
			switch kind {
			case "other-container":
				f.app.ID = "foreign"
			case "other-image":
				f.app.Image = targetImage
			case "mount":
				f.app.Mounts = threeMounts()
			case "pending":
				submitTest(t, o)
			case "unknown":
				submitTest(t, o)
				o.S.Current.Phase = "recovery_required"
			}
			if e := o.refresh(""); e == nil {
				t.Fatal("unresolved context trusted")
			}
			if !bytes.Equal(old, jsonBytes(o.D.Rec)) || f.up != 0 {
				t.Fatal("refusal changed registration")
			}
		})
	}
}
func TestWorkflowQuietIdleCallCounts(t *testing.T) {
	for _, mode := range []string{"adopted", "unavailable", "unadopted"} {
		t.Run(mode, func(t *testing.T) {
			o, f, now := fixture(t)
			if mode == "adopted" {
				submitTest(t, o)
				drive(t, o, now)
			}
			if mode == "unavailable" {
				f.daemonDown = true
			}
			if mode == "unadopted" {
				o.D.Rec.Integrated = false
			}
			f.calls = nil
			f.mutations = nil
			f.statusWrites = nil
			start := *now
			for i := 0; i < 60; i++ {
				o.tick()
				*now++
			}
			direct, mutating := 0, 0
			for n := range f.calls {
				if f.mutations[n] {
					mutating++
				} else {
					direct++
				}
			}
			// Baseline performs the same full work every 2 seconds and journals status.
			perBeat := 5
			if mode == "unavailable" {
				perBeat = 2
			}
			if mode == "unadopted" {
				perBeat = 4
			}
			if len(f.calls) != 12*perBeat || mutating != 0 {
				t.Fatalf("calls=%d direct=%d mutation=%d", len(f.calls), direct, mutating)
			}
			var s Snapshot
			if e := load(filepath.Join(o.Root, "status.json"), &s); e != nil || s.Heartbeat < start+55 {
				t.Fatal("heartbeat stale", e)
			}
			t.Logf("fake minute %s: before=%d calls/30 journaled status writes; after=%d calls/0 journaled status writes; heartbeat=5s", mode, 30*perBeat, len(f.calls))
		})
	}
}
func TestWorkflowNativeEntryUsesTestedLaunchAndResume(t *testing.T) {
	b, e := os.ReadFile("main.go")
	if e != nil {
		t.Fatal(e)
	}
	s := string(b)
	for _, want := range []string{"o.launchSelected(time.Now", "o.resumeSelection()", `case "refresh":`, "release()", `runMain([]string{"wait"`, "o.LastWork = lastWork"} {
		if !strings.Contains(s, want) {
			t.Fatal("native entry bypass", want)
		}
	}
	for _, name := range []string{"integration.go", "../install.sh", "../install.ps1", "../start.sh", "../installers/mac/install-mac.sh", "../installers/windows/install-vectoraiz.ps1", "../deploy/owner/setup-owner.sh", "../deploy/owner/setup-owner.ps1"} {
		b, e := os.ReadFile(name)
		if e != nil || !strings.Contains(string(b), "launch --root") {
			t.Fatal("launcher does not reach owner", name, e)
		}
	}
	if allowNativeExecution {
		t.Fatal("guard open")
	}
}

func TestWorkflowResolverWithoutBuildxAndOptionalMetadata(t *testing.T) {
	for _, kind := range []string{"normal", "single-no-platform", "oci", "index", "ambiguous", "wrong-platform", "bad-config", "changed-immutable-descriptor"} {
		t.Run(kind, func(t *testing.T) {
			o, f, _ := fixture(t)
			o.D.R = workflowRunner{f, func(a []string, b []byte, d int64, m int, v bool) ([]byte, error, bool) {
				if a[3] != "manifest" {
					return nil, nil, false
				}
				if v {
					t.Fatal("resolver requested mutation")
				}
				out, e := f.Run(a, b, d, m, v)
				if e != nil {
					return out, e, true
				}
				var doc map[string]any
				json.Unmarshal(out, &doc)
				descriptor := doc["Descriptor"].(map[string]any)
				immutable := strings.Contains(a[len(a)-1], "@")
				switch kind {
				case "single-no-platform":
					delete(descriptor, "platform")
				case "oci":
					doc["OCIManifest"] = doc["SchemaV2Manifest"]
					delete(doc, "SchemaV2Manifest")
				case "index":
					if !immutable {
						other := map[string]any{"Descriptor": map[string]any{"digest": indexDigest, "platform": map[string]string{"os": "linux", "architecture": "amd64"}}}
						return jsonBytes([]any{other, doc}), nil, true
					}
				case "ambiguous":
					if !immutable {
						return jsonBytes([]any{doc, doc}), nil, true
					}
				case "wrong-platform":
					descriptor["platform"] = map[string]string{"os": "linux", "architecture": "amd64"}
				case "bad-config":
					doc["SchemaV2Manifest"] = map[string]any{"config": map[string]string{"digest": "mutable"}}
				case "changed-immutable-descriptor":
					if immutable {
						descriptor["digest"] = indexDigest
					}
				}
				return jsonBytes(doc), nil, true
			}}
			r, e := o.D.resolve("v2.0.0")
			good := kind == "normal" || kind == "single-no-platform" || kind == "oci" || kind == "index"
			if (e == nil) != good {
				t.Fatal(kind, r, e)
			}
			if good {
				f.pulled = true
				if r.Digest != targetDigest || r.ImageID != targetImage || o.D.image(r) != nil {
					t.Fatal("immutable/platform/config proof lost", r)
				}
			}
			for _, a := range f.calls {
				if strings.Contains(strings.Join(a, " "), "buildx") {
					t.Fatal("required buildx")
				}
			}
		})
	}
}
func TestWorkflowDesktopSelectionNeverStartsRemoteRuntime(t *testing.T) {
	o, _, _ := fixture(t)
	for _, c := range []struct{ platform, host, want string }{
		{"darwin", "unix:///var/run/docker.sock", "Docker"},
		{"darwin", "unix:///Users/person/.orbstack/run/docker.sock", "OrbStack"},
		{"darwin", "unix:///Users/person/.docker/run/docker.sock", "Docker"},
		{"windows", "npipe:////./pipe/docker_engine", "Docker Desktop"},
		{"darwin", "ssh://remote-B", ""}, {"windows", "tcp://remote-B:2376", ""}, {"linux", "unix:///var/run/docker.sock", ""},
	} {
		o.D.Rec.Endpoint = jsonBytes([]any{map[string]any{"Endpoints": map[string]any{"docker": map[string]string{"Host": c.host}}}})
		if got := o.D.desktopRuntimeFor(c.platform); got != c.want {
			t.Fatal(c, got)
		}
	}
	o.D.Rec.Invocation.Context = ""
	o.D.Rec.Invocation.Environment = append(o.D.Rec.Invocation.Environment, "DOCKER_HOST=ssh://B")
	if got := o.D.desktopRuntimeFor("darwin"); got != "" {
		t.Fatal("host override started wrong local runtime", got)
	}
}
func TestWorkflowExplicitEquivalentContextAndForeignControl(t *testing.T) {
	for _, foreign := range []bool{false, true} {
		t.Run(map[bool]string{false: "equivalent", true: "foreign"}[foreign], func(t *testing.T) {
			o, f, now := fixture(t)
			submitTest(t, o)
			drive(t, o, now)
			selected := *o.S.Desired
			o.D.R = workflowRunner{f, func(a []string, b []byte, d int64, m int, v bool) ([]byte, error, bool) {
				if a[2] != "explicit-B" {
					return nil, nil, false
				}
				copyArgs := append([]string{}, a...)
				copyArgs[2] = "remote-B"
				out, e := f.Run(copyArgs, b, d, m, v)
				if foreign && a[3] == "inspect" {
					var ids []Identity
					json.Unmarshal(out, &ids)
					ids[0].ID = "foreign"
					out = jsonBytes(ids)
				}
				return out, e, true
			}}
			e := o.refresh("explicit-B")
			if foreign {
				if e == nil || o.D.Rec.Invocation.Context != "remote-B" {
					t.Fatal("foreign context adopted", e)
				}
				return
			}
			if e != nil || o.D.Rec.Invocation.Context != "explicit-B" || *o.S.Desired != selected {
				t.Fatal("equivalent explicit context failed", e)
			}
		})
	}
}
func TestWorkflowLaunchUnknownCannotReplay(t *testing.T) {
	o, f, now := fixture(t)
	submitTest(t, o)
	drive(t, o, now)
	f.app.State.Running = false
	f.partialFail = true
	e := o.launchSelected(func() time.Time { return time.Unix(*now, 0) }, func() { *now++ }, func() error { return nil })
	if e == nil || o.S.Current.Phase != "recovery_required" || !o.S.Current.ActivationIssued {
		t.Fatal("launch mutation uncertainty lost", e)
	}
	up := f.up
	restarted, e := openOwner(o.Root, o.D)
	if e != nil {
		t.Fatal(e)
	}
	restarted.Now = o.Now
	if e = restarted.step(); e != nil || f.up != up {
		t.Fatal("launch activation replayed", e)
	}
}

func TestWorkflowRevalidateObservationOutageBeforeDependencies(t *testing.T) {
	for _, phase := range []string{"accepted", "downloading", "downloaded", "activating", "verifying"} {
		for _, branch := range []string{"config", "context"} {
			t.Run(phase+"/"+branch, func(t *testing.T) {
				o, f, now := fixture(t)
				reachWorkflowPhase(t, o, now, phase)
				down := true
				blocked := 0
				o.D.R = workflowRunner{f, func(a []string, b []byte, d int64, m int, v bool) ([]byte, error, bool) {
					match := branch == "context" && a[3] == "context" || branch == "config" && a[3] == "compose" && strings.Contains(strings.Join(a, " "), "config --format")
					if down && match {
						if v {
							t.Fatal("read-only outage became mutation")
						}
						blocked++
						return nil, errors.New("command_exit"), true
					}
					return nil, nil, false
				}}
				deadline := o.S.Current.Deadline
				up := f.up
				for n := 0; n < 2; n++ {
					if e := o.step(); e != nil {
						t.Fatal(e)
					}
					*now++
				}
				if blocked != 2 || o.S.Current.Phase != phase || o.S.Current.Deadline != deadline || f.up != up {
					t.Fatal("pre-dependency observation failed permanently", o.S.Current)
				}
				down = false
				drive(t, o, now)
				if o.S.Current.Phase != "succeeded" || f.up != 1 {
					t.Fatal("did not resume after observation restored", o.S.Current)
				}
			})
		}
	}
}

func TestWorkflowMutationFailureIsNeverObservationWaiting(t *testing.T) {
	o, f, now := fixture(t)
	reachWorkflowPhase(t, o, now, "downloaded")
	f.dep.State.Running = false
	starts := 0
	o.D.R = workflowRunner{f, func(a []string, b []byte, d int64, m int, v bool) ([]byte, error, bool) {
		if v && strings.Contains(strings.Join(a, " "), "start postgres") {
			starts++
			return nil, errors.New("child_outcome_unknown"), true
		}
		return nil, nil, false
	}}
	if e := o.step(); e != nil {
		t.Fatal(e)
	}
	if o.S.Current.Phase != "recovery_required" || starts != 1 {
		t.Fatal("dependency mutation uncertainty masked", o.S.Current)
	}
	o.step()
	if starts != 1 {
		t.Fatal("uncertain dependency command replayed")
	}
}
func TestWorkflowLaunchInspectionErrorCannotAuthorizeActivation(t *testing.T) {
	o, f, now := fixture(t)
	submitTest(t, o)
	drive(t, o, now)
	up := f.up
	o.D.R = workflowRunner{f, func(a []string, b []byte, d int64, m int, v bool) ([]byte, error, bool) {
		if strings.Contains(strings.Join(a, " "), "config --hash vectoraiz") {
			return nil, errors.New("command_output_limit"), true
		}
		return nil, nil, false
	}}
	if e := o.launchSelected(func() time.Time { return time.Unix(*now, 0) }, func() { *now++ }, func() error { return nil }); e == nil || f.up != up {
		t.Fatal("failed inspection authorized activation", e)
	}
}
