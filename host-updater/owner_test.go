package main

import (
	"archive/tar"
	"bytes"
	"crypto/sha256"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
	"time"
)

func TestMain(m *testing.M) {
	allowNativeExecution = false
	fmt.Println("ISOLATION: native process execution denied; stdlib fake runner only; no sockets/network/application imports")
	os.Exit(m.Run())
}

const targetDigest = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
const targetImage = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
const indexDigest = "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
const oldImage = "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"

type fakeDocker struct {
	t                                                                                *testing.T
	root                                                                             string
	config                                                                           []byte
	app, dep                                                                         Identity
	boxes                                                                            map[string]map[string][]byte
	calls                                                                            [][]string
	mutations                                                                        []bool
	pulled                                                                           bool
	up, pulls, starts                                                                int
	wrongImage, wrongVersion, unhealthy, noWriter, pullFail, partialFail, daemonDown bool
	ackDurable                                                                       bool
	owner                                                                            *Owner
	drift                                                                            bool
	resolved                                                                         int
}

func jsonBytes(v any) []byte { b, _ := json.Marshal(v); return b }

// Resolve the fake's actual original service and final override, rather than
// returning a target hash for every query. Hashes model configuration identity,
// not Docker's hash algorithm (real Compose remains NOT_RUN).
func (f *fakeDocker) composeConfig(selected bool) map[string]any {
	var config map[string]any
	if e := json.Unmarshal(f.config, &config); e != nil {
		f.t.Fatal(e)
	}
	if selected {
		var override struct{ Services map[string]map[string]any }
		if e := load(filepath.Join(f.root, "selection.compose.json"), &override); e != nil {
			f.t.Fatal(e)
		}
		services := config["services"].(map[string]any)
		for name, fields := range override.Services {
			s := services[name].(map[string]any)
			for k, v := range fields {
				if k == "environment" {
					for key, value := range v.(map[string]any) {
						s[k].(map[string]any)[key] = value
					}
				} else {
					s[k] = v
				}
			}
		}
	}
	return config
}
func (f *fakeDocker) configHash(service string, selected bool) string {
	s := f.composeConfig(selected)["services"].(map[string]any)[service]
	return fmt.Sprintf("%x", sha256.Sum256(jsonBytes(s)))
}
func ownedIdentity(id, image, service string) Identity {
	c := Identity{ID: id, Image: image}
	c.Config.Image = repository + ":v1.0.0"
	c.Config.Env = []string{"VECTORAIZ_VERSION=v1.0.0", "PASSWORD=private-sentinel"}
	c.Config.Labels = map[string]string{"com.docker.compose.project": "custom-project", "com.docker.compose.service": service, "com.docker.compose.config-hash": "original-" + service, "version": "v1.0.0"}
	c.HostConfig.RestartPolicy = map[string]any{"Name": "unless-stopped", "MaximumRetryCount": float64(0)}
	c.Mounts = []map[string]any{{"Type": "volume", "Name": service + "-retained", "Destination": "/data", "RW": true}}
	c.State.Running = true
	c.State.Health.Status = "healthy"
	return c
}
func (f *fakeDocker) Run(args []string, input []byte, deadline int64, max int, mutate bool) ([]byte, error) {
	f.calls = append(f.calls, append([]string{}, args...))
	f.mutations = append(f.mutations, mutate)
	if args[0] != "/installed/docker" {
		f.t.Fatalf("product required RTK or unexpected binary: %v", args)
	}
	if deadline <= 0 || max <= 0 {
		f.t.Fatal("unbounded process")
	}
	if f.daemonDown {
		return nil, errors.New("daemon_down")
	}
	a := args[3:]
	if len(args) < 4 || args[1] != "--context" || args[2] != "remote-B" {
		f.t.Fatalf("lost context: %v", args)
	}
	switch a[0] {
	case "context":
		return []byte(`[{"Endpoints":{"docker":{"Host":"ssh://already-authorized-B"}}}]`), nil
	case "info":
		return []byte("linux/arm64"), nil
	case "inspect":
		c := f.app
		if a[1] == f.dep.ID {
			c = f.dep
		}
		if a[1] != f.app.ID && a[1] != f.dep.ID {
			return nil, errors.New("unknown_container")
		}
		return jsonBytes([]Identity{c}), nil
	case "buildx":
		if strings.Contains(strings.Join(a, " "), "--format") {
			f.resolved++
			return jsonBytes(map[string]string{"digest": indexDigest}), nil
		}
		if strings.Contains(strings.Join(a, " "), "@"+indexDigest) {
			return []byte(`{"manifests":[{"digest":"` + targetDigest + `","platform":{"os":"linux","architecture":"arm64"}}]}`), nil
		}
		return []byte(`{"config":{"digest":"` + targetImage + `"}}`), nil
	case "image":
		if !f.pulled {
			return nil, errors.New("uncached")
		}
		return jsonBytes([]any{map[string]any{"Id": targetImage, "RepoDigests": []string{repository + "@" + indexDigest, repository + "@" + targetDigest}, "Os": "linux", "Architecture": "arm64", "Config": map[string]any{"Labels": map[string]string{"version": "v2.0.0"}}}}), nil
	case "pull":
		var visible Snapshot
		if load(filepath.Join(f.root, "status.json"), &visible) != nil || visible.Current == nil || visible.Current.Phase != "downloading" {
			f.t.Fatal("host status did not publish download intent before long pull")
		}
		f.pulls++
		if f.pullFail {
			return nil, errors.New("pull_failed")
		}
		if a[len(a)-1] != repository+"@"+targetDigest {
			f.t.Fatalf("mutable pull: %v", a)
		}
		f.pulled = true
		return nil, nil
	case "exec":
		v := "v2.0.0"
		if f.wrongVersion {
			v = "v1.0.0"
		}
		return jsonBytes(map[string]string{"status": "ok", "version": v}), nil
	case "cp":
		if a[1] == "-" {
			dest := strings.SplitN(a[2], ":", 2)[0]
			tr := tar.NewReader(bytes.NewReader(input))
			h, e := tr.Next()
			if e != nil {
				return nil, e
			}
			b, e := unpack(input, h.Name, 32768)
			if e != nil {
				return nil, e
			}
			if f.boxes[dest] == nil {
				f.boxes[dest] = map[string][]byte{}
			}
			f.boxes[dest][h.Name] = b
			if h.Name == "bootstrap.json" && !f.noWriter {
				var p Pair
				json.Unmarshal(b, &p)
				ready, _ := sign(p.Key, "ready", p)
				f.boxes[dest]["ready.json"] = ready
			}
			if h.Name == "ack.json" {
				var state State
				if e = load(filepath.Join(f.root, "journal.json"), &state); e != nil || state.Current == nil {
					f.t.Fatal("ack before durable journal")
				}
				f.ackDurable = true
			}
			return nil, nil
		}
		parts := strings.SplitN(a[1], ":", 2)
		name := filepath.Base(parts[1])
		b := f.boxes[parts[0]][name]
		if b == nil {
			return nil, errors.New("absent_mailbox")
		}
		return pack(name, b)
	case "compose":
		selected := false
		for _, s := range a {
			if s == filepath.Join(f.root, "selection.compose.json") {
				selected = true
			}
		}
		command := ""
		for _, s := range a {
			if s == "config" || s == "ps" || s == "up" || s == "start" {
				command = s
				break
			}
		}
		switch command {
		case "config":
			if a[len(a)-1] == "*" {
				return []byte("vectoraiz " + f.configHash("vectoraiz", selected) + "\npostgres " + f.configHash("postgres", selected) + "\n"), nil
			}
			if a[len(a)-1] == "vectoraiz" {
				return []byte("vectoraiz " + f.configHash("vectoraiz", selected) + "\n"), nil
			}
			b := jsonBytes(f.composeConfig(selected))
			if f.drift {
				b = bytes.ReplaceAll(b, []byte("private-sentinel"), []byte("changed-password"))
			}
			return b, nil
		case "ps":
			if a[len(a)-1] == "postgres" {
				return []byte(f.dep.ID), nil
			}
			return []byte(f.app.ID), nil
		case "start":
			f.starts++
			f.dep.State.Running = true
			f.dep.State.Health.Status = "healthy"
			return nil, nil
		case "up":
			if a[len(a)-1] == "--help" {
				return []byte("--no-deps --no-build --pull"), nil
			}
			f.up++
			if !selected || !strings.Contains(strings.Join(a, " "), "up -d --no-deps --no-build --pull never vectoraiz") {
				f.t.Fatalf("unsafe activation argv %v", a)
			}
			f.app.ID = "new-container-0000000000000000"
			f.app.Image = targetImage
			if f.wrongImage {
				f.app.Image = oldImage
			}
			service := f.composeConfig(true)["services"].(map[string]any)["vectoraiz"].(map[string]any)
			f.app.Config.Image = service["image"].(string)
			f.app.Config.Env = nil
			for k, v := range service["environment"].(map[string]any) {
				f.app.Config.Env = append(f.app.Config.Env, k+"="+v.(string))
			}
			f.app.Config.Labels["version"] = "v2.0.0"
			f.app.Config.Labels["com.docker.compose.config-hash"] = f.configHash("vectoraiz", true)
			f.app.State.Running = true
			if f.unhealthy {
				f.app.State.Health.Status = "unhealthy"
			}
			if f.partialFail {
				return nil, errors.New("compose_nonzero_after_recreation")
			}
			return nil, nil
		}
	}
	f.t.Fatalf("unexpected product argv %v", args)
	return nil, errors.New("unexpected_argv")
}
func fixture(t *testing.T) (*Owner, *fakeDocker, *int64) {
	return fixtureWithConfig(t, false)
}
func fixtureWithConfig(t *testing.T, sameRelease bool) (*Owner, *fakeDocker, *int64) {
	t.Helper()
	root := t.TempDir()
	f := &fakeDocker{t: t, root: root, app: ownedIdentity("old-container-0000000000000000", oldImage, "vectoraiz"), dep: ownedIdentity("dependency-000000000000000000", oldImage, "postgres"), boxes: map[string]map[string][]byte{}}
	f.config = []byte(`{"name":"custom-project","services":{"vectoraiz":{"image":"` + repository + `:v1.0.0","environment":{"VECTORAIZ_VERSION":"v1.0.0","PASSWORD":"private-sentinel"},"restart":"unless-stopped"},"postgres":{"image":"postgres:16","environment":{"PASSWORD":"private-sentinel"}}}}`)
	if sameRelease {
		f.config = bytes.ReplaceAll(f.config, []byte("v1.0.0"), []byte("v2.0.0"))
		f.app.Image = targetImage
		f.app.Config.Image = repository + ":v2.0.0"
		f.app.Config.Env = []string{"VECTORAIZ_VERSION=v2.0.0", "PASSWORD=private-sentinel"}
		f.app.Config.Labels["version"] = "v2.0.0"
		f.pulled = true
	}
	f.app.Config.Labels["com.docker.compose.config-hash"] = f.configHash("vectoraiz", false)
	f.dep.Config.Labels["com.docker.compose.config-hash"] = f.configHash("postgres", false)
	source := filepath.Join(root, "original compose ñ.json")
	os.WriteFile(source, f.config, 0600)
	env := filepath.Join(root, "original.env")
	os.WriteFile(env, []byte("PASSWORD=private-sentinel\n"), 0600)
	rec, e := register(root, Invocation{Docker: "/installed/docker", Cwd: root, Files: []string{source}, EnvFiles: []string{env}, Context: "remote-B", Channel: "direct", Service: "vectoraiz", Environment: []string{"HOME=" + root, "VECTORAIZ_VERSION=v1.0.0", "PRIVATE=private-sentinel"}}, f)
	if e != nil {
		t.Fatal(e)
	}
	rec.Integrated = true
	d := &Docker{R: f, Rec: rec, Root: root}
	if e = d.pair(f.app, 0); e != nil {
		t.Fatal(e)
	}
	o, e := openOwner(root, d)
	if e != nil {
		t.Fatal(e)
	}
	now := time.Now().Unix()
	o.Now = func() int64 { return now }
	o.S.Limits = Limits{10, 20, 12, 18, 1, 2}
	f.owner = o
	return o, f, &now
}
func submitTest(t *testing.T, o *Owner) Ack {
	t.Helper()
	a, e := o.submit(Message{Key: "stable-key-0000000000", Target: "v2.0.0"})
	if e != nil || a.Code != 202 {
		t.Fatalf("submit: %+v %v", a, e)
	}
	return a
}
func drive(t *testing.T, o *Owner, now *int64) {
	t.Helper()
	for n := 0; n < 40 && o.S.Current.active(); n++ {
		if e := o.tick(); e != nil {
			t.Fatal(e)
		}
		*now++
	}
	if o.S.Current.active() {
		t.Fatal("never terminal")
	}
}

func TestPinnedOldReplacementAndPersistence(t *testing.T) {
	o, f, now := fixture(t)
	submitTest(t, o)
	drive(t, o, now)
	if o.S.Current.Phase != "succeeded" || f.up != 1 || f.pulls != 1 || f.app.Image != targetImage || o.S.LastVerified == nil {
		t.Fatalf("not actually activated %+v pulls=%d up=%d", o.S.Current, f.pulls, f.up)
	}
	var selection struct {
		Release    Release
		Generation int
	}
	if e := load(filepath.Join(o.Root, "selection.json"), &selection); e != nil || selection.Release.Digest != targetDigest {
		t.Fatal("lost selection")
	}
	if f.dep.ID != o.D.Rec.Containers["postgres"].ID {
		t.Fatal("dependency recreated")
	}
	if e := o.D.revalidate(); e != nil {
		t.Fatal(e)
	}
	b, _ := o.D.effective(true)
	if !bytesEqualJSON(b, o.D.Rec.Effective) {
		t.Fatal("changed private effective config")
	}
	calls := len(f.calls)
	o2, e := openOwner(o.Root, o.D)
	if e != nil {
		t.Fatal(e)
	}
	o2.Now = o.Now
	if e = o2.step(); e != nil {
		t.Fatal(e)
	}
	if len(f.calls) != calls || f.up != 1 {
		t.Fatal("completed operation reactivated")
	}
}
func TestCausalOldRestartDoesNotActivate(t *testing.T) {
	o, f, now := fixture(t)
	submitTest(t, o)
	f.wrongImage = true
	drive(t, o, now)
	if o.S.Current.Phase != "recovery_required" || o.S.LastVerified != nil || f.app.Image != oldImage {
		t.Fatalf("old image falsely succeeded: %+v", o.S.Current)
	}
}
func TestCausalHTTP200OldVersionDoesNotSucceed(t *testing.T) {
	o, f, now := fixture(t)
	submitTest(t, o)
	f.wrongVersion = true
	drive(t, o, now)
	if o.S.Current.Phase != "recovery_required" || o.S.LastVerified != nil {
		t.Fatal("old reported version falsely succeeded")
	}
}
func TestFailuresPreserveSelectionAndRecovery(t *testing.T) {
	for _, kind := range []string{"pull", "partial", "unhealthy", "no-writer", "drift"} {
		t.Run(kind, func(t *testing.T) {
			o, f, now := fixture(t)
			switch kind {
			case "pull":
				f.pullFail = true
			case "partial":
				f.partialFail = true
			case "unhealthy":
				f.unhealthy = true
			case "no-writer":
				f.noWriter = true
			case "drift":
				f.drift = true
			}
			submitTest(t, o)
			drive(t, o, now)
			if o.S.Current.Phase == "succeeded" || o.S.LastVerified != nil {
				t.Fatal("false success")
			}
			if kind == "pull" || kind == "drift" {
				if o.S.Desired != nil || f.up != 0 || f.app.Image != oldImage {
					t.Fatal("mutated before failed preflight/download")
				}
			} else {
				if o.S.Current.Phase != "recovery_required" {
					t.Fatalf("wrong terminal %+v", o.S.Current)
				}
				a, _ := o.submit(Message{Key: "new-key-000000000000", Target: "v3.0.0"})
				if a.Code != 409 {
					t.Fatal("unreconciled recovery accepted mutation")
				}
			}
			if !reflect.DeepEqual(f.dep, o.D.Rec.Containers["postgres"]) {
				t.Fatal("dependency configuration changed")
			}
		})
	}
}
func TestIdempotencyDurableAckAndLostResponse(t *testing.T) {
	o, f, now := fixture(t)
	p := o.D.Rec.Pair
	m := Message{Kind: "submit", Installation: p.Installation, Epoch: p.Epoch, Container: p.Container, Generation: p.Generation, Key: "same-lost-key-00000", Target: "v2.0.0", Principal: "authenticated-user", Scope: "update:submit", Nonce: "nonce-00000000000000", Expires: *now + 90}
	b, _ := sign(p.Key, "request", m)
	f.boxes[p.Container]["request.json"] = b
	if e := o.tick(); e != nil {
		t.Fatal(e)
	}
	if !f.ackDurable {
		t.Fatal("no durable ack")
	}
	id := o.S.Current.ID
	o2, e := openOwner(o.Root, o.D)
	if e != nil {
		t.Fatal(e)
	}
	o2.Now = o.Now
	a, e := o2.submit(m)
	if e != nil || a.ID != id {
		t.Fatal("duplicate after lost response")
	}
	m.Target = "v3.0.0"
	a, _ = o2.submit(m)
	if a.Code != 409 {
		t.Fatal("same key target conflict accepted")
	}
	m.Key = "different-key-000000"
	m.Target = "v2.0.0"
	a, _ = o2.submit(m)
	if a.ID != id || a.Code != 202 {
		t.Fatal("same target duplicated")
	}
	m.Key = "different-key-111111"
	m.Target = "v3.0.0"
	a, _ = o2.submit(m)
	if a.Code != 409 {
		t.Fatal("concurrent target accepted")
	}
}
func TestProtocolIdentityMACExpiryAndBounds(t *testing.T) {
	p := Pair{Installation: token(), Epoch: token(), Container: token(), Key: token(), Generation: 4}
	base := Message{Kind: "submit", Installation: p.Installation, Epoch: p.Epoch, Container: p.Container, Generation: 4, Key: token(), Target: "v2.0.0", Principal: "principal", Scope: "update:submit", Nonce: token(), Expires: 110}
	for _, kind := range []string{"valid", "mac", "epoch", "container", "generation", "scope", "expired", "future", "principal", "target"} {
		t.Run(kind, func(t *testing.T) {
			m := base
			key := p.Key
			switch kind {
			case "mac":
				key = "wrong"
			case "epoch":
				m.Epoch = token()
			case "container":
				m.Container = token()
			case "generation":
				m.Generation++
			case "scope":
				m.Scope = "host:exec"
			case "expired":
				m.Expires = 99
			case "future":
				m.Expires = 999
			case "principal":
				m.Principal = ""
			case "target":
				m.Target = "latest"
			}
			b, _ := sign(key, "request", m)
			_, e := request(p, b, 100)
			if (e == nil) != (kind == "valid") {
				t.Fatalf("%s %v", kind, e)
			}
		})
	}
	if _, e := request(p, bytes.Repeat([]byte("x"), 4097), 100); e == nil {
		t.Fatal("unbounded request")
	}
}
func TestArchiveRejectsPartialLinksExtensionsExtraAndTraversal(t *testing.T) {
	valid, _ := pack("request.json", []byte(`{"ok":true}`))
	if _, e := unpack(valid, "request.json", 4096); e != nil {
		t.Fatal(e)
	}
	cases := map[string][]byte{"truncated": valid[:len(valid)-512], "oversize": make([]byte, 66048)}
	for _, kind := range []byte{tar.TypeSymlink, tar.TypeLink, tar.TypeChar, tar.TypeXHeader} {
		v := append([]byte{}, valid...)
		v[156] = kind
		cases[fmt.Sprintf("type-%d", kind)] = v
	}
	for _, name := range []string{"../request.json", "/request.json", "dir/request.json", "ack.json"} {
		var b bytes.Buffer
		tw := tar.NewWriter(&b)
		tw.WriteHeader(&tar.Header{Name: name, Size: 2, Mode: 0600})
		tw.Write([]byte("{}"))
		tw.Close()
		cases[name] = b.Bytes()
	}
	extra := append([]byte{}, valid...)
	extra[len(extra)-1] = 1
	cases["trailing"] = extra
	for name, b := range cases {
		if _, e := unpack(b, "request.json", 4096); e == nil {
			t.Fatalf("accepted %s", name)
		}
	}
}
func TestMailboxMountReadOnlyAndActualWriter(t *testing.T) {
	for _, kind := range []string{"volume-run", "tmpfs-run", "readonly", "nonroot", "writable", "no-writer"} {
		t.Run(kind, func(t *testing.T) {
			o, f, _ := fixture(t)
			c := f.app
			switch kind {
			case "volume-run":
				c.Mounts = append(c.Mounts, map[string]any{"Destination": "/run"})
			case "tmpfs-run":
				c.HostConfig.Tmpfs = map[string]string{"/run": "rw"}
			case "readonly":
				c.HostConfig.ReadonlyRootfs = true
			case "nonroot":
				c.Config.User = "1000"
			case "no-writer":
				f.noWriter = true
			}
			e := o.D.pair(c, 1)
			if kind == "no-writer" {
				if e != nil {
					t.Fatal(e)
				}
				if o.D.challenge() == nil {
					t.Fatal("advertised without real writer challenge")
				}
			} else if (e == nil) != (kind == "writable") {
				t.Fatalf("%s %v", kind, e)
			}
		})
	}
}
func TestDependencyStoppedStartOnlyThenReadiness(t *testing.T) {
	o, f, now := fixture(t)
	f.dep.State.Running = false
	f.dep.State.Health.Status = ""
	submitTest(t, o)
	drive(t, o, now)
	if f.starts != 1 || f.up != 1 || o.S.Current.Phase != "succeeded" || f.dep.ID != o.D.Rec.Containers["postgres"].ID {
		t.Fatal("dependency not reconciled safely")
	}
	for _, args := range f.calls {
		joined := strings.Join(args, " ")
		if strings.Contains(joined, "pull postgres") || strings.Contains(joined, "up -d postgres") || strings.Contains(joined, " down") {
			t.Fatal("dependency recreated or stopped")
		}
	}
}
func TestEffectiveDriftAndHarmlessFormatting(t *testing.T) {
	o, _, _ := fixture(t)
	for p := range o.D.Rec.Sources {
		os.WriteFile(p, []byte("# harmless reformat, fake resolver returns unchanged effective config\n"), 0600)
	}
	if e := o.D.revalidate(); e != nil {
		t.Fatal("harmless bytes rejected", e)
	}
	for p := range o.D.Rec.Sources {
		os.Chmod(p, 0644)
		break
	}
	if e := o.D.revalidate(); e == nil {
		t.Fatal("ownership/mode change ignored")
	}
}
func TestCrashSelectionRepairAndNoDuplicateActivation(t *testing.T) {
	o, f, now := fixture(t)
	submitTest(t, o)
	for o.S.Current.Phase != "activating" {
		if e := o.tick(); e != nil {
			t.Fatal(e)
		}
		*now++
	}
	o.D.selectRelease(o.S.Current.Release, o.S.Generation)
	os.Remove(filepath.Join(o.Root, "selection.compose.json"))
	o2, e := openOwner(o.Root, o.D)
	if e != nil {
		t.Fatal(e)
	}
	o2.Now = o.Now
	drive(t, o2, now)
	if f.up != 1 || o2.S.Current.Phase != "succeeded" {
		t.Fatal("selector recovery failed")
	}
	o3, f3, n3 := fixture(t)
	submitTest(t, o3)
	for o3.S.Current.Phase != "activating" {
		o3.tick()
		*n3++
	}
	o3.S.Current.ActivationIssued = true
	o3.persist()
	if e = o3.step(); e != nil {
		t.Fatal(e)
	}
	if f3.up != 0 || o3.S.Current.Phase != "recovery_required" {
		t.Fatal("unknown activation replayed")
	}
}
func TestCrashDoesNotRenewBudgetAndOutageStatus(t *testing.T) {
	o, f, now := fixture(t)
	submitTest(t, o)
	deadline := o.S.Current.Deadline
	*now += 100
	o2, e := openOwner(o.Root, o.D)
	if e != nil {
		t.Fatal(e)
	}
	o2.Now = o.Now
	f.daemonDown = true
	if e = o2.tick(); e == nil { /* expected mailbox publication may fail */
	}
	if o2.S.Current.Deadline != deadline || o2.S.Current.active() {
		t.Fatal("renewed crash budget")
	}
	var s Snapshot
	if e = load(filepath.Join(o.Root, "status.json"), &s); e != nil || s.Current == nil {
		t.Fatal("no independent outage status")
	}
	raw, _ := os.ReadFile(filepath.Join(o.Root, "status.json"))
	if bytes.Contains(raw, []byte("private-sentinel")) || bytes.Contains(raw, []byte(o.D.Rec.Pair.Key)) {
		t.Fatal("secret leaked")
	}
}
func TestOSLockAndNativeExecutionGuard(t *testing.T) {
	root := t.TempDir()
	f, e := lock(filepath.Join(root, "lock"))
	if e != nil {
		t.Fatal(e)
	}
	if g, e := lock(filepath.Join(root, "lock")); e == nil {
		unlock(g)
		t.Fatal("concurrent lock acquired")
	}
	unlock(f)
	g, e := lock(filepath.Join(root, "lock"))
	if e != nil {
		t.Fatal(e)
	}
	unlock(g)
	if e = execute(&Command{Args: []string{"docker", "info"}}); e == nil || !strings.Contains(e.Error(), "ISOLATION_GUARD") {
		t.Fatal("native guard open")
	}
	t.Log("GUARD_EVENT: native Docker process blocked before spawn")
}
func TestOriginalOrderAndLiteralOwnerOverride(t *testing.T) {
	o, _, _ := fixture(t)
	i := &o.D.Rec.Invocation
	i.Files = []string{"/A/path ñ/base.yml", "/A/override.yml"}
	i.EnvFiles = []string{"/A/one.env", "/A/two.env"}
	i.ProjectDirectory = "/A/project directory"
	args := o.D.compose(true, "up", "-d", "--no-deps", "--no-build", "--pull", "never", "vectoraiz")
	want := []string{"compose", "--project-directory", i.ProjectDirectory, "-p", "custom-project", "-f", i.Files[0], "-f", i.Files[1], "--env-file", i.EnvFiles[0], "--env-file", i.EnvFiles[1], "-f", filepath.Join(o.Root, "selection.compose.json"), "up", "-d", "--no-deps", "--no-build", "--pull", "never", "vectoraiz"}
	if !reflect.DeepEqual(args, want) {
		t.Fatalf("lost invocation order %v", args)
	}
}
func TestRejectedRequestNeverBecomesAQueuedUpdate(t *testing.T) {
	o, _, now := fixture(t)
	submitTest(t, o)
	m := Message{Key: "rejected-key-00000000", Target: "v3.0.0"}
	a, e := o.submit(m)
	if e != nil || a.Code != 409 {
		t.Fatal("no conflict")
	}
	o.S.Current.Phase = "succeeded"
	o.persist()
	o2, e := openOwner(o.Root, o.D)
	if e != nil {
		t.Fatal(e)
	}
	o2.Now = o.Now
	*now++
	a, e = o2.submit(m)
	if e != nil || a.Code != 409 || o2.S.Current.Target != "v2.0.0" {
		t.Fatal("rejected request became queued mutation")
	}
}
func TestUncertainChildAndExpiredDeadlineAreNotReplayed(t *testing.T) {
	root := t.TempDir()
	path := filepath.Join(root, "child.json")
	before := Command{ID: token(), Args: []string{"docker", "compose", "up"}, Deadline: time.Now().Unix() - 10, Started: true}
	if e := save(path, before); e != nil {
		t.Fatal(e)
	}
	p := Processes{Root: root}
	if e := p.Reconcile(); e == nil || e.Error() != "child_may_be_running" {
		t.Fatal("uncertain child assumed complete", e)
	}
	var after Command
	load(path, &after)
	if !reflect.DeepEqual(before, after) {
		t.Fatal("uncertain command mutated")
	}
	if _, e := p.Run([]string{"docker", "info"}, nil, time.Now().Unix()+10, 100, true); e == nil || !strings.Contains(e.Error(), "ISOLATION_GUARD") {
		t.Fatal("process adapter could spawn under fake guard")
	}
}
func TestHostStatusCLIHasNoDaemonOrMutation(t *testing.T) {
	o, f, _ := fixture(t)
	submitTest(t, o)
	o.publish()
	f.daemonDown = true
	before, _ := os.ReadFile(filepath.Join(o.Root, "journal.json"))
	calls := len(f.calls)
	if e := runMain([]string{"status", "--root", o.Root}); e != nil {
		t.Fatal(e)
	}
	after, _ := os.ReadFile(filepath.Join(o.Root, "journal.json"))
	if !bytes.Equal(before, after) || len(f.calls) != calls {
		t.Fatal("status mutated or called daemon")
	}
}
func TestMailboxCapabilityMissingReadyIsNeverAdvertised(t *testing.T) {
	o, f, _ := fixture(t)
	f.boxes[o.D.Rec.Pair.Container]["ready.json"] = []byte(`{"payload":`)
	o.tick()
	if o.snapshot().Ready {
		t.Fatal("ready advertised with partial/no writer echo")
	}
	f.boxes[o.D.Rec.Pair.Container]["ready.json"] = nil
	o.tick()
	if o.snapshot().Ready {
		t.Fatal("ready advertised on unavailable path")
	}
}
func TestForwardRepairOrderingAndNoSchemaDowngrade(t *testing.T) {
	for _, v := range []struct {
		from, to string
		valid    bool
	}{{"v2.0.0", "v2.0.0", true}, {"v2.0.0", "v2.0.1", true}, {"v2.0.0", "v1.9.9", false}, {"v2.0.0", "v2.0.0-rc.1", false}, {"v2.0.0-rc.1", "v2.0.0", true}} {
		if forwardTag(v.from, v.to) != v.valid {
			t.Fatal(v)
		}
	}
}

func TestRegisteredAlternativeUnderPreservedRunMount(t *testing.T) {
	c := ownedIdentity("container", oldImage, "vectoraiz")
	c.HostConfig.Tmpfs = map[string]string{"/run": "rw"}
	p, e := chooseMailbox(c)
	if e != nil || p != "/var/lib/vectoraiz-owner-mailbox" {
		t.Fatal("arbitrary /run-only refusal", p, e)
	}
	c.Mounts = append(c.Mounts, map[string]any{"Destination": "/var"})
	p, e = chooseMailbox(c)
	if e != nil || p != "/opt/vectoraiz-owner-mailbox" {
		t.Fatal("did not retain custom mounts with alternate image path", p, e)
	}
	c.HostConfig.ReadonlyRootfs = true
	if _, e = chooseMailbox(c); e == nil {
		t.Fatal("advertised writable layer on read-only root")
	}
}

func TestReadOnlyInspectionDoesNotEnterMutationHandoff(t *testing.T) {
	o, f, now := fixture(t)
	submitTest(t, o)
	drive(t, o, now)
	for n, args := range f.calls {
		a := args[3:]
		expected := a[0] == "pull" || a[0] == "cp" && a[1] == "-"
		if a[0] == "compose" {
			for _, v := range a {
				if v == "up" && a[len(a)-1] != "--help" || v == "start" {
					expected = true
				}
			}
		}
		if f.mutations[n] != expected {
			t.Fatalf("wrong mutation handoff: %v mutate=%v", a, f.mutations[n])
		}
	}
}
func TestReplacementMustAnswerBeforeReadyAdvertisement(t *testing.T) {
	o, f, _ := fixture(t)
	f.app.ID = "replacement-container-00000000"
	f.noWriter = true
	if e := o.tick(); e != nil {
		t.Fatal(e)
	}
	if o.snapshot().Ready {
		t.Fatal("replacement advertised ready before signed reader/writer challenge")
	}
}
func TestOrphanRecoveryRequiresHelperLockAndAbsentProcessGroup(t *testing.T) {
	for _, alive := range []bool{true, false} {
		t.Run(fmt.Sprint(alive), func(t *testing.T) {
			root := t.TempDir()
			r := Command{ID: token(), Started: true, ProcessGroup: 12345, Deadline: time.Now().Unix() - 10, Args: []string{"private-command"}, Env: []string{"SECRET=private"}}
			save(filepath.Join(root, "child.json"), r)
			p := Processes{Root: root, GroupAlive: func(pid int) bool {
				if pid != 12345 {
					t.Fatal("wrong tracked group")
				}
				return alive
			}}
			guard, e := lock(filepath.Join(root, "child.lock"))
			if e != nil {
				t.Fatal(e)
			}
			if e = p.Reconcile(); e == nil || e.Error() != "child_may_be_running" {
				t.Fatal("ignored live helper lock", e)
			}
			unlock(guard)
			e = p.Reconcile()
			var got Command
			load(filepath.Join(root, "child.json"), &got)
			if alive {
				if e == nil || got.Done {
					t.Fatal("unresolved live descendants cleared")
				}
			} else {
				if e == nil || e.Error() != "child_outcome_unknown" || !got.Done || got.Args != nil || got.Env != nil {
					t.Fatal("dead helper not recoverable/redacted", e, got)
				}
				if e = p.Reconcile(); e != nil {
					t.Fatal("explicit forward recovery permanently blocked", e)
				}
			}
		})
	}
}
func TestActualForwardRepairRetainsFailedHistoryAndVerifiesTarget(t *testing.T) {
	o, f, now := fixture(t)
	submitTest(t, o)
	f.partialFail = true
	drive(t, o, now)
	failedID := o.S.Current.ID
	if o.S.Current.Phase != "recovery_required" {
		t.Fatal("expected partial replacement failure")
	}
	if e := o.forwardRepair("v1.0.0"); e == nil {
		t.Fatal("schema downgrade accepted")
	}
	f.partialFail = false
	if e := o.forwardRepair("v2.0.0"); e != nil {
		t.Fatal(e)
	}
	if o.S.Current.ID == failedID || o.S.LastVerified != nil {
		t.Fatal("reused failed outcome")
	}
	drive(t, o, now)
	if o.S.Current.Phase != "succeeded" || f.up != 1 || f.pulls != 1 || len(o.S.History) != 1 || o.S.History[0].ID != failedID {
		t.Fatal("forward reconciliation duplicated activation or lost history")
	}
}

func TestMissingHelperBeforeCommandStartIsRecoverableWithoutNativeObservation(t *testing.T) {
	root := t.TempDir()
	save(filepath.Join(root, "child.json"), Command{ID: token(), Deadline: time.Now().Unix() - 10})
	p := Processes{Root: root, GroupAlive: func(int) bool { t.Fatal("not-started helper must not inspect native identity"); return true }}
	if e := p.Reconcile(); e == nil || e.Error() != "child_outcome_unknown" {
		t.Fatal("lost pre-start handoff blocked all future recovery", e)
	}
	if e := p.Reconcile(); e != nil {
		t.Fatal(e)
	}
}
func TestWrongProjectAndConfigHashCannotVerifyEvenHealthyTarget(t *testing.T) {
	for _, kind := range []string{"project", "config"} {
		t.Run(kind, func(t *testing.T) {
			o, f, now := fixture(t)
			submitTest(t, o)
			for o.S.Current.Phase != "verifying" {
				if e := o.tick(); e != nil {
					t.Fatal(e)
				}
				*now++
			}
			if kind == "project" {
				f.app.Config.Labels["com.docker.compose.project"] = "unrelated"
			} else {
				f.app.Config.Labels["com.docker.compose.config-hash"] = "unrelated"
			}
			drive(t, o, now)
			if o.S.Current.Phase == "succeeded" || o.S.LastVerified != nil {
				t.Fatal("healthy but unrelated configuration falsely succeeded")
			}
		})
	}
}

func TestTerminalNotificationCannotPrecedeDurableIndependentStatus(t *testing.T) {
	o, f, now := fixture(t)
	notifications := 0
	o.Notify = func(root string) {
		notifications++
		var state State
		var visible Snapshot
		if load(filepath.Join(root, "journal.json"), &state) != nil || load(filepath.Join(root, "status.json"), &visible) != nil || visible.Current == nil || visible.Current.active() || state.NotificationID != visible.Current.ID {
			t.Fatal("notification before durable terminal status")
		}
	}
	submitTest(t, o)
	f.partialFail = true
	drive(t, o, now)
	if notifications != 1 {
		t.Fatal("terminal notice not attempted once")
	}
	o.persist()
	if notifications != 1 {
		t.Fatal("notification replayed instead of retaining independent status")
	}
}
