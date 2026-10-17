package main

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"sort"
	"time"
)

type Limits struct {
	Resolve, Pull, Activate, Verify, Interval int64
	Samples                                   int
}

func defaults() Limits { return Limits{60, 1800, 180, 600, 5, 3} }
func (l Limits) valid() bool {
	return l.Resolve > 0 && l.Resolve <= 600 && l.Pull > 0 && l.Pull <= 7200 && l.Activate > 0 && l.Activate <= 1200 && l.Verify > 0 && l.Verify <= 3600 && l.Interval > 0 && l.Interval <= 30 && l.Samples > 0 && l.Samples <= 20
}

type Operation struct {
	ID                string  `json:"id"`
	Target            string  `json:"target"`
	Phase             string  `json:"phase"`
	Code              string  `json:"code,omitempty"`
	Deadline          int64   `json:"deadline"`
	Updated           int64   `json:"updated"`
	Generation        int     `json:"generation"`
	Release           Release `json:"release"`
	Prior             string  `json:"prior"`
	PriorImage        string  `json:"prior_image"`
	ObservedImage     string  `json:"observed_image,omitempty"`
	ObservedContainer string  `json:"observed_container,omitempty"`
	Samples           int     `json:"samples"`
	NextSample        int64   `json:"next_sample"`
	ActivationIssued  bool    `json:"activation_issued"`
	Limits            Limits  `json:"limits"`
}

func (o *Operation) active() bool {
	return o != nil && o.Phase != "succeeded" && o.Phase != "failed_download" && o.Phase != "failed_resolution" && o.Phase != "recovery_required"
}

type Ack struct {
	Key    string `json:"key"`
	Target string `json:"target"`
	ID     string `json:"id"`
	Code   int    `json:"code"`
}
type State struct {
	Generation     int
	Current        *Operation
	History        []*Operation
	Keys           map[string]Ack
	LastRefusal    Ack
	LastVerified   *Release
	Desired        *Release
	Limits         Limits
	NotificationID string
}
type Snapshot struct {
	Installation string         `json:"installation"`
	Epoch        string         `json:"epoch"`
	Container    string         `json:"container"`
	Generation   int            `json:"generation"`
	Heartbeat    int64          `json:"heartbeat"`
	Ready        bool           `json:"ready"`
	Reason       string         `json:"reason"`
	Current      *Operation     `json:"current"`
	History      []*Operation   `json:"history"`
	Keys         map[string]Ack `json:"keys"`
	LastVerified *Release       `json:"last_verified"`
	Recovery     string         `json:"recovery"`
}
type Owner struct {
	Root          string
	D             *Docker
	S             State
	Now           func() int64
	LastWork      int64
	LastPublished int64
	Reason        string
	Notify        func(string)
}

func openOwner(root string, d *Docker) (*Owner, error) {
	o := &Owner{Root: root, D: d, Now: func() int64 { return time.Now().Unix() }}
	e := load(filepath.Join(root, "journal.json"), &o.S)
	if os.IsNotExist(e) {
		o.S = State{Keys: map[string]Ack{}, Limits: defaults()}
		return o, o.persist()
	}
	if e == nil && (!o.S.Limits.valid() || o.S.Current != nil && !o.S.Current.Limits.valid() || len(o.S.Keys) > 32 || len(o.S.History) > 6) {
		return nil, errors.New("invalid_journal_limits")
	}
	return o, e
}
func (o *Owner) persist() error {
	notify := o.S.Current != nil && !o.S.Current.active() && o.S.NotificationID != o.S.Current.ID
	if notify {
		o.S.NotificationID = o.S.Current.ID
	}
	if e := save(filepath.Join(o.Root, "journal.json"), &o.S); e != nil {
		return e
	}
	// Publish each durable phase before the next potentially long command. The
	// host file remains useful even if the app disappears or the owner crashes.
	if e := o.hostStatus(); e != nil {
		return e
	}
	if notify {
		if o.Notify != nil {
			o.Notify(o.Root)
		} else {
			notifyTerminal(o.Root)
		}
	}
	return nil
}
func (o *Owner) snapshot() Snapshot {
	p := o.D.Rec.Pair
	return Snapshot{Installation: o.D.Rec.Installation, Epoch: p.Epoch, Container: p.Container, Generation: p.Generation, Heartbeat: o.Now(), Ready: o.D.Rec.Integrated && o.Reason == "", Reason: o.Reason, Current: o.S.Current, History: o.S.History, Keys: o.S.Keys, LastVerified: o.S.LastVerified, Recovery: "On the original Compose client: vectoraiz-owner status --root <installation>/.vectoraiz-owner. The status.json file remains readable without the app, owner or Docker. Original restart policy is retained; a failed app may keep restarting. No automatic rollback or database downgrade."}
}
func (o *Owner) hostStatus() error {
	s := o.snapshot()
	b, e := json.Marshal(s)
	if e != nil {
		return e
	}
	if len(b) > 32768 {
		return errors.New("snapshot_size")
	}
	if e = atomicFile(filepath.Join(o.Root, "status.json"), b); e != nil {
		return e
	}
	o.LastPublished = o.Now()
	return nil
}
func (o *Owner) publish() error {
	if e := o.hostStatus(); e != nil {
		return e
	}
	s := o.snapshot()
	var e error
	if o.D.Rec.Pair.Container != "" {
		var b []byte
		b, e = sign(o.D.Rec.Pair.Key, "status", s)
		if e == nil {
			e = o.D.writeBox(o.D.Rec.Pair.Container, "status.json", b)
		}
	}
	return e
}
func (o *Owner) submit(m Message) (Ack, error) {
	if a, ok := o.S.Keys[m.Key]; ok {
		if a.Target != m.Target {
			return Ack{Key: m.Key, Target: m.Target, ID: a.ID, Code: 409}, nil
		}
		return a, nil
	}
	if o.S.LastRefusal.Key == m.Key {
		a := o.S.LastRefusal
		if a.Target != m.Target {
			a.Target = m.Target
			a.Code = 409
		}
		return a, nil
	}
	refuse := func(code int) (Ack, error) {
		a := Ack{Key: m.Key, Target: m.Target, Code: code}
		if o.S.Current != nil {
			a.ID = o.S.Current.ID
		}
		o.S.LastRefusal = a
		if len(o.S.Keys) < 32 {
			o.S.Keys[m.Key] = a
		}
		return a, o.persist()
	}
	if o.S.Current != nil && o.S.Current.Phase == "recovery_required" {
		return refuse(409)
	}
	if len(o.S.Keys) >= 32 && !o.S.Current.active() { // Terminal key aliases have finite bounded retention.
		keys := make([]string, 0, len(o.S.Keys))
		for k := range o.S.Keys {
			keys = append(keys, k)
		}
		sort.Strings(keys)
		for _, k := range keys[:8] {
			delete(o.S.Keys, k)
		}
	}
	if len(o.S.Keys) >= 32 {
		return refuse(429)
	}
	a := Ack{Key: m.Key, Target: m.Target, Code: 202}
	if o.S.Current.active() {
		a.ID = o.S.Current.ID
		if o.S.Current.Target != m.Target {
			return refuse(409)
		}
	} else {
		if o.S.Current != nil {
			o.S.History = append(o.S.History, o.S.Current)
		}
		o.S.Generation++
		o.S.Current = &Operation{ID: token(), Target: m.Target, Phase: "accepted", Generation: o.S.Generation, Updated: o.Now(), Deadline: o.Now() + o.S.Limits.Resolve, Limits: o.S.Limits}
		a.ID = o.S.Current.ID
	}
	o.S.Keys[m.Key] = a
	o.trim()
	return a, o.persist() // Durable before any ack or execution.
}
func (o *Owner) trim() {
	if len(o.S.History) > 6 {
		o.S.History = o.S.History[len(o.S.History)-6:]
	}
	keep := map[string]bool{}
	if o.S.Current != nil {
		keep[o.S.Current.ID] = true
	}
	for _, h := range o.S.History {
		keep[h.ID] = true
	}
	for k, a := range o.S.Keys {
		if !keep[a.ID] {
			delete(o.S.Keys, k)
		}
	}
}
func (o *Owner) phase(p string, budget int64) error {
	c := o.S.Current
	c.Phase = p
	c.Updated = o.Now()
	c.Deadline = o.Now() + budget
	c.Samples = 0
	return o.persist()
}
func (o *Owner) fail(code string) error {
	c := o.S.Current
	if c == nil {
		return nil
	}
	c.Code = code
	c.Updated = o.Now()
	switch c.Phase {
	case "accepted", "resolving":
		c.Phase = "failed_resolution"
	case "downloading":
		c.Phase = "failed_download"
	default:
		c.Phase = "recovery_required"
	}
	return o.persist()
}
func (o *Owner) step() error {
	c := o.S.Current
	if !c.active() {
		return nil
	}
	o.D.Until = c.Deadline
	if o.Now() >= c.Deadline {
		o.D.Until = o.Now() + 15
		code := "phase_deadline"
		if c.Phase == "verifying" && c.Code != "daemon_observation_waiting" {
			code = "startup_failed_migration_unconfirmed"
			if b, e := o.D.readBox(o.D.Rec.Pair.Container, "startup.json", 4096); e == nil {
				var event struct{ Phase, Version string }
				if json.Unmarshal(b, &event) == nil && event.Phase == "migration_failed" && event.Version == c.Target {
					code = "migration_failed"
				}
			}
		}
		return o.fail(code)
	}
	// Read-only observation failures can wait; helper/handoff uncertainty and
	// verified configuration drift are never recast as daemon downtime.
	if e := o.D.revalidate(); e != nil {
		if observationWaiting(e) {
			return o.waitObservation()
		}
		return o.fail(e.Error())
	}
	if _, e := o.D.run(false, nil, 32768, "info", "--format", "{{.OSType}}/{{.Architecture}}"); e != nil {
		if observationWaiting(e) {
			return o.waitObservation()
		}
		return o.fail(e.Error())
	}
	if c.Code == "daemon_observation_waiting" {
		c.Code = ""
	}
	switch c.Phase {
	case "accepted":
		c.Phase = "resolving"
		c.Updated = o.Now()
		return o.persist()
	case "resolving":
		from := o.D.Rec.Containers[o.D.Rec.Invocation.Service].Config.Labels["version"]
		if o.S.Desired != nil {
			from = o.S.Desired.Tag
		}
		if tagPattern.MatchString(from) && !forwardTag(from, c.Target) {
			return o.fail("backward_schema_selection_refused")
		}
		r, e := o.D.resolve(c.Target)
		if e != nil {
			return o.fail("release_unresolved")
		}
		c.Release = r
		old, e := o.D.container(o.D.Rec.Invocation.Service)
		if observationWaiting(e) {
			return o.waitObservation()
		}
		if e != nil {
			return o.fail("original_app_unresolved")
		}
		c.Prior = old.ID
		c.PriorImage = old.Image
		return o.phase("downloading", c.Limits.Pull)
	case "downloading":
		// Inspect cached immutable image before replaying a pull after a crash.
		if o.D.image(c.Release) != nil {
			if e := o.D.pull(c.Release, c.Deadline); e != nil {
				return o.fail("download_failed")
			}
			if e := o.D.image(c.Release); e != nil {
				return o.fail("download_identity_failed")
			}
		}
		return o.phase("downloaded", c.Limits.Activate)
	case "downloaded":
		if e := o.D.dependencies(); e != nil {
			if observationWaiting(e) {
				return o.waitObservation()
			}
			if e.Error() == "dependencies_waiting" {
				return nil
			}
			return o.fail("dependency_recovery_required")
		}
		// Journal desired selection BEFORE publishing either selector file. A crash
		// repairs both from this same journal under the shared installation lock.
		o.S.Desired = &c.Release
		if e := o.phase("activating", c.Limits.Activate); e != nil {
			return e
		}
		return nil
	case "activating":
		if e := o.D.selectRelease(c.Release, c.Generation); e != nil {
			return e
		}
		effective, e := o.D.effective(true)
		if observationWaiting(e) {
			return o.waitObservation()
		}
		if e != nil || !bytesEqualJSON(effective, o.D.Rec.Effective) {
			return o.fail("selection_configuration_changed")
		}
		if e = o.D.dependencies(); e != nil {
			if observationWaiting(e) {
				return o.waitObservation()
			}
			if e.Error() == "dependencies_waiting" {
				return nil
			}
			return o.fail("dependency_recovery_required")
		}
		current, e := o.D.container(o.D.Rec.Invocation.Service)
		if observationWaiting(e) {
			return o.waitObservation()
		}
		if e == nil {
			selectedErr := o.D.selectedActive(current, c.Release)
			if observationWaiting(selectedErr) {
				return o.waitObservation()
			}
			if selectedErr == nil {
				return o.phase("verifying", c.Limits.Verify)
			}
			if !selectionNeedsActivation(selectedErr) {
				return o.fail(selectedErr.Error())
			}
		}
		if c.ActivationIssued {
			return o.fail("activation_outcome_requires_reconciliation")
		}
		if e != nil || current.ID != c.Prior || !sameRuntime(o.D.Rec.Containers[o.D.Rec.Invocation.Service], current) {
			return o.fail("unexpected_activation_identity")
		}
		c.ActivationIssued = true
		if e = o.persist(); e != nil {
			return e
		}
		_, e = o.D.comp(true, true, "up", "-d", "--no-deps", "--no-build", "--pull", "never", o.D.Rec.Invocation.Service)
		// Inspect even on nonzero: partial recreation can already have migrated.
		actual, ie := o.D.container(o.D.Rec.Invocation.Service)
		if ie == nil {
			c.ObservedContainer = actual.ID
			c.ObservedImage = actual.Image
		}
		if e != nil {
			return o.fail("compose_exit_recovery_required")
		}
		return o.phase("verifying", c.Limits.Verify)
	case "verifying":
		if o.Now() < c.NextSample {
			return nil
		}
		if e := o.D.dependencies(); e != nil {
			if observationWaiting(e) {
				return o.waitObservation()
			}
			if e.Error() == "dependencies_waiting" {
				c.Samples = 0
				return o.persist()
			}
			return o.fail("dependency_recovery_required")
		}
		current, e := o.D.container(o.D.Rec.Invocation.Service)
		if e != nil {
			c.Samples = 0
			return o.persist()
		}
		c.ObservedContainer = current.ID
		c.ObservedImage = current.Image
		if current.Image != c.Release.ImageID {
			return o.fail("wrong_image")
		}
		if current.ID != o.D.Rec.Pair.Container || o.D.Rec.Pair.Generation != c.Generation {
			if e = o.D.pair(current, c.Generation); e != nil {
				return o.fail(e.Error())
			}
		}
		if e = o.D.challenge(); e != nil {
			c.Samples = 0
			c.NextSample = o.Now() + c.Limits.Interval
			return o.persist()
		}
		_, e = o.D.observed(c.Release, c.Prior, c.PriorImage)
		if e != nil {
			c.Samples = 0
			c.Code = e.Error()
			c.NextSample = o.Now() + c.Limits.Interval
			return o.persist()
		}
		c.Code = ""
		c.Samples++
		c.NextSample = o.Now() + c.Limits.Interval
		c.Updated = o.Now()
		if c.Samples >= c.Limits.Samples {
			c.Phase = "succeeded"
			o.S.LastVerified = &c.Release
		}
		return o.persist()
	default:
		return o.fail("unknown_phase")
	}
	return nil
}
func (o *Owner) tick() error {
	// Active phases keep their one-second scheduler and absolute deadlines.
	// Quiet observations/mailbox/status publication share the five-second beat.
	if !o.S.Current.active() && o.LastWork != 0 && o.Now()-o.LastWork < 5 {
		return nil
	}
	o.LastWork = o.Now()
	o.D.Until = 0
	if o.S.Current.active() {
		if o.Now() >= o.S.Current.Deadline {
			if e := o.step(); e != nil {
				return e
			}
			o.D.Until = 0
			return o.publish()
		}
		o.D.Until = o.S.Current.Deadline
	}
	// Re-pair after a host restart only to an owned known image. Never trust HOSTNAME.
	c, e := o.D.container(o.D.Rec.Invocation.Service)
	if e == nil {
		base := o.D.Rec.Containers[o.D.Rec.Invocation.Service]
		allowed := c.Image == base.Image || o.S.Desired != nil && c.Image == o.S.Desired.ImageID
		if !allowed || !sameRuntime(base, c) {
			o.Reason = "context_unresolved"
		} else if o.D.Rec.Pair.Container != c.ID {
			if e = o.D.pair(c, o.S.Generation); e != nil {
				o.Reason = e.Error()
			} else if o.D.challenge() != nil {
				o.Reason = "channel_not_integrated"
			} else {
				o.Reason = ""
			}
		} else if o.D.challenge() != nil {
			o.Reason = "channel_not_integrated"
		} else {
			o.Reason = ""
		}
	} else {
		o.Reason = "owner_daemon_unavailable"
	}
	if o.Reason == "" && o.D.Rec.Integrated {
		if b, re := o.D.readBox(o.D.Rec.Pair.Container, "request.json", 4096); re == nil {
			if m, re := request(o.D.Rec.Pair, b, o.Now()); re == nil {
				a, se := o.submit(m)
				if se != nil {
					return se
				}
				ack := struct {
					Pair Pair `json:"pair"`
					Ack  Ack  `json:"ack"`
				}{o.D.Rec.Pair, a}
				signed, _ := sign(o.D.Rec.Pair.Key, "ack", ack)
				o.D.writeBox(o.D.Rec.Pair.Container, "ack.json", signed)
			}
		}
	}
	if e = o.step(); e != nil {
		return e
	}
	o.D.Until = 0
	return o.publish()
}

func (o *Owner) forwardRepair(target string) error {
	var e error
	if !tagPattern.MatchString(target) {
		return errors.New("exact_forward_repair_target_required")
	}
	if o.S.Current == nil || o.S.Current.Phase != "recovery_required" {
		return errors.New("no_recovery_operation")
	}
	if !forwardTag(o.S.Current.Target, target) {
		return errors.New("automatic_or_backward_downgrade_refused")
	}
	if e = o.D.revalidate(); e != nil {
		return e
	}
	c, e := o.D.container(o.D.Rec.Invocation.Service)
	if e != nil {
		return e
	}
	if !sameRuntime(o.D.Rec.Containers[o.D.Rec.Invocation.Service], c) || (c.Image != o.D.Rec.Containers[o.D.Rec.Invocation.Service].Image && (o.S.Desired == nil || c.Image != o.S.Desired.ImageID)) {
		return errors.New("actual_state_unresolved")
	}
	o.S.Current.ObservedImage = c.Image
	o.S.Current.ObservedContainer = c.ID
	o.S.History = append(o.S.History, o.S.Current)
	o.S.Current = nil
	_, e = o.submit(Message{Key: token(), Target: target})
	return e
}
