package main

import (
	"errors"
	"os"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"
	"time"
)

func TestMaterialSameImageSelectedConfigurationAndRunningState(t *testing.T) {
	for _, kind := range []string{"fresh-adoption", "changed-mailbox", "stopped-target", "already-active", "issued-already-active", "issued-wrong-config", "unrelated-container"} {
		t.Run(kind, func(t *testing.T) {
			o, f, now := fixtureWithConfig(t, true)
			submitTest(t, o)
			for o.S.Current.Phase != "activating" {
				if e := o.tick(); e != nil {
					t.Fatal(e)
				}
				*now++
			}
			c := o.S.Current
			if e := o.D.selectRelease(c.Release, c.Generation); e != nil {
				t.Fatal(e)
			}
			originalHash := f.configHash("vectoraiz", false)
			selectedHash := f.configHash("vectoraiz", true)
			if originalHash == selectedHash {
				t.Fatal("fake failed to distinguish actual original/selected config")
			}
			if kind != "fresh-adoption" && kind != "issued-wrong-config" && kind != "unrelated-container" {
				f.app.Config.Labels["com.docker.compose.config-hash"] = selectedHash
				f.app.Config.Env = append(f.app.Config.Env, "VECTORAIZ_OWNER_MAILBOX="+o.D.mailboxPath())
			}
			if kind == "changed-mailbox" {
				// An actual different previously selected environment identity.
				var previous map[string]any
				load(filepath.Join(o.Root, "selection.compose.json"), &previous)
				service := previous["services"].(map[string]any)["vectoraiz"].(map[string]any)
				service["environment"].(map[string]any)["VECTORAIZ_OWNER_MAILBOX"] = "/opt/vectoraiz-owner-mailbox"
				save(filepath.Join(o.Root, "selection.compose.json"), previous)
				f.app.Config.Labels["com.docker.compose.config-hash"] = f.configHash("vectoraiz", true)
				f.app.Config.Env[len(f.app.Config.Env)-1] = "VECTORAIZ_OWNER_MAILBOX=/opt/vectoraiz-owner-mailbox"
				if f.app.Config.Labels["com.docker.compose.config-hash"] == selectedHash {
					t.Fatal("mailbox config change invisible")
				}
			}
			if kind == "stopped-target" {
				f.app.State.Running = false
			}
			if strings.HasPrefix(kind, "issued-") {
				c.ActivationIssued = true
				o.persist()
			}
			if kind == "unrelated-container" {
				f.app.ID = "unrelated-container-00000000"
			}
			drive(t, o, now)
			if kind == "issued-wrong-config" || kind == "unrelated-container" {
				if f.up != 0 || c.Phase != "recovery_required" {
					t.Fatal("unknown command/provenance replayed", c)
				}
				return
			}
			wantUp := 1
			if kind == "already-active" || kind == "issued-already-active" {
				wantUp = 0
			}
			if c.Phase != "succeeded" || f.up != wantUp || f.pulls != 0 || o.S.LastVerified == nil {
				t.Fatalf("same-image outcome phase=%s code=%s up=%d want=%d", c.Phase, c.Code, f.up, wantUp)
			}
			if f.up > 0 && !c.ActivationIssued {
				t.Fatal("activation not journaled")
			}
			if f.app.Config.Labels["com.docker.compose.config-hash"] != selectedHash || !f.app.State.Running {
				t.Fatal("actual config not activated")
			}
		})
	}
}

func TestMaterialSameReleaseForwardRepairStartsStoppedSelectedTarget(t *testing.T) {
	o, f, now := fixture(t)
	submitTest(t, o)
	f.partialFail = true
	drive(t, o, now)
	failed := o.S.Current.ID
	if o.S.Current.Phase != "recovery_required" || f.up != 1 || f.app.Image != targetImage {
		t.Fatal("partial activation fixture did not reach selected target")
	}
	f.app.State.Running = false
	f.partialFail = false
	if e := o.forwardRepair("v2.0.0"); e != nil {
		t.Fatal(e)
	}
	drive(t, o, now)
	if o.S.Current.Phase != "succeeded" || f.up != 2 || !f.app.State.Running || len(o.S.History) != 1 || o.S.History[0].ID != failed {
		t.Fatal("same-release forward repair skipped stopped target or lost history", o.S.Current)
	}
}

type fakeHelper struct {
	startError, waitError   error
	entered, finish, reaped chan struct{}
	waits                   atomic.Int32
}

func (f *fakeHelper) Start() error { return f.startError }
func (f *fakeHelper) Wait() error {
	f.waits.Add(1)
	close(f.entered)
	<-f.finish
	close(f.reaped)
	return f.waitError
}
func awaitFake(t *testing.T, done <-chan struct{}) {
	t.Helper()
	select {
	case <-done:
	case <-time.After(time.Second):
		t.Fatal("fake lifecycle did not progress")
	}
}
func TestMaterialEveryStartedHelperReapedWithoutBlockingOwner(t *testing.T) {
	for _, outcome := range []string{"normal", "nonzero", "deadline-group-terminated"} {
		t.Run(outcome, func(t *testing.T) {
			// Repeated mailbox-sized helper handoffs, all fake: no OS children.
			helpers := make([]*fakeHelper, 64)
			for n := range helpers {
				f := &fakeHelper{entered: make(chan struct{}), finish: make(chan struct{}), reaped: make(chan struct{})}
				if outcome != "normal" {
					f.waitError = errors.New(outcome)
				}
				helpers[n] = f
				if e := startHelper(f); e != nil {
					t.Fatal(e)
				}
				awaitFake(t, f.entered)
				// Startup returned and Wait is owned even while helper is live.
				if f.waits.Load() != 1 {
					t.Fatal("missing/duplicate wait")
				}
			}
			for _, f := range helpers {
				close(f.finish)
				awaitFake(t, f.reaped)
			}
			for _, f := range helpers {
				if f.waits.Load() != 1 {
					t.Fatal("helper not reaped exactly once")
				}
			}
		})
	}
}
func TestMaterialFailedStartDoesNotWait(t *testing.T) {
	f := &fakeHelper{startError: errors.New("failed-start")}
	if e := startHelper(f); e == nil || f.waits.Load() != 0 {
		t.Fatal("failed start waited")
	}
}
func TestMaterialDurableResultIndependentOfLiveReaper(t *testing.T) {
	root := t.TempDir()
	f := &fakeHelper{entered: make(chan struct{}), finish: make(chan struct{}), reaped: make(chan struct{})}
	if e := startHelper(f); e != nil {
		t.Fatal(e)
	}
	awaitFake(t, f.entered)
	// Polling can finish/restart independently; Wait never authorizes mutation.
	r := Command{ID: token(), Started: true, Done: true, Deadline: time.Now().Unix() - 10, Code: "command_exit"}
	if e := save(filepath.Join(root, "child.json"), r); e != nil {
		t.Fatal(e)
	}
	p := Processes{Root: root, GroupAlive: func(int) bool { t.Fatal("completed outcome inspected native group"); return true }}
	if e := p.Reconcile(); e != nil {
		t.Fatal(e)
	}
	var got Command
	load(filepath.Join(root, "child.json"), &got)
	if got.Code != r.Code {
		t.Fatal("reaper replaced durable outcome")
	}
	close(f.finish)
	awaitFake(t, f.reaped)
}
func TestMaterialRealAdapterWiresReaperAndRetainsDetachedBudget(t *testing.T) {
	raw, e := os.ReadFile("process.go")
	if e != nil {
		t.Fatal(e)
	}
	src := string(raw)
	if !strings.Contains(src, "startHelper(c)") || strings.Contains(src, "c.Process.Release()") {
		t.Fatal("real adapter bypasses reaper")
	}
	if !strings.Contains(src, "childGroup(c)") || !strings.Contains(src, "context.WithDeadline") || !strings.Contains(src, "c.WaitDelay = 2 * time.Second") {
		t.Fatal("lost detached finite budget")
	}
	unix, e := os.ReadFile("os_unix.go")
	if e != nil {
		t.Fatal(e)
	}
	if strings.Contains(string(unix), "Pdeathsig") || !strings.Contains(string(unix), "Setpgid: true") {
		t.Fatal("helper lifetime tied to owner")
	}
	t.Log("STATIC_ONLY: real Start/Wait wiring, detached group and finite budgets; native supervision NOT_RUN")
}
