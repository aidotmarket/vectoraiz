package main

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"time"
)

func observationWaiting(e error) bool {
	if e == nil {
		return false
	}
	switch e.Error() {
	case "command_exit", "command_deadline", "daemon_down":
		return true
	}
	return false
}

func selectionNeedsActivation(e error) bool {
	if e == nil {
		return false
	}
	switch e.Error() {
	case "wrong_image", "startup_waiting", "actual_app_configuration_mismatch":
		return true
	}
	return false
}

// Validate local sources and the pinned endpoint before starting any desktop.
// A remote context never causes a local runtime to start.
func (d *Docker) desktopRuntime() string { return d.desktopRuntimeFor(runtime.GOOS) }
func (d *Docker) desktopRuntimeFor(platform string) string {
	if d.Rec.Invocation.Context == "" {
		for _, v := range d.Rec.Invocation.Environment {
			if strings.HasPrefix(v, "DOCKER_HOST=") && v != "DOCKER_HOST=" {
				return ""
			}
		}
	}
	var endpoints []struct {
		Endpoints map[string]struct{ Host string }
	}
	if json.Unmarshal(d.Rec.Endpoint, &endpoints) != nil || len(endpoints) != 1 {
		return ""
	}
	host := endpoints[0].Endpoints["docker"].Host
	if platform == "darwin" && strings.HasPrefix(host, "unix://") {
		if strings.Contains(host, "/.orbstack/") {
			return "OrbStack"
		}
		if host == "unix:///var/run/docker.sock" || strings.Contains(host, "/.docker/run/docker.sock") {
			return "Docker"
		}
	}
	if platform == "windows" && strings.HasPrefix(host, "npipe://") {
		return "Docker Desktop"
	}
	return ""
}
func (d *Docker) startRuntime() error {
	switch d.desktopRuntime() {
	case "OrbStack":
		return native("open", "-a", "OrbStack")
	case "Docker":
		if _, e := os.Stat("/Applications/Docker.app"); os.IsNotExist(e) {
			if _, e = os.Stat("/Applications/OrbStack.app"); e == nil {
				return native("open", "-a", "OrbStack")
			}
		}
		return native("open", "-a", "Docker")
	case "Docker Desktop":
		// Use the normal installed Desktop executable, with existing user privileges.
		return native("powershell.exe", "-NoProfile", "-NonInteractive", "-Command", `Start-Process -FilePath (Join-Path $env:ProgramFiles 'Docker\Docker\Docker Desktop.exe')`)
	}
	return nil
}
func (d *Docker) waitAvailable(until int64, now func() time.Time, pause func(), start func() error) error {
	d.Until = until
	started := false
	for {
		if now().Unix() >= until {
			return errors.New("daemon_wait_deadline")
		}
		if e := d.revalidate(); e != nil {
			if !observationWaiting(e) {
				return e
			}
		} else {
			if _, e := d.run(false, nil, 32768, "info", "--format", "{{.OSType}}/{{.Architecture}}"); e == nil {
				return nil
			} else if !observationWaiting(e) {
				return e
			}
			if !started {
				// revalidate succeeded, including original context inspection.
				started = true
				if e := start(); e != nil {
					return e
				}
			}
		}
		pause()
	}
}

// Status is finalized before a pending launch releases the owner lock to wait.
func (o *Owner) launchStatus(now func() time.Time, pause func()) (func(), func(error) error, error) {
	o.Reason = "launch_waiting"
	if e := o.hostStatus(); e != nil {
		return nil, nil, e
	}
	finish := func(result error) error {
		o.Reason = ""
		if result != nil {
			o.Reason = result.Error()
		}
		if e := o.hostStatus(); result == nil {
			return e
		}
		return result
	}
	return func() {
		pause()
		if now().Unix()-o.LastPublished >= 5 {
			_ = o.hostStatus()
		}
	}, finish, nil
}
func (o *Owner) launchSelected(now func() time.Time, pause func(), start func() error) (result error) {
	pause, finish, e := o.launchStatus(now, pause)
	if e != nil {
		return e
	}
	defer func() { result = finish(result) }()
	d := o.D
	deadline := now().Unix() + o.S.Limits.Activate
	if e := d.waitAvailable(deadline, now, pause, start); e != nil {
		return e
	}
	if e := d.selectRelease(*o.S.Desired, o.S.Generation); e != nil {
		return e
	}
	for {
		if now().Unix() >= deadline {
			return errors.New("launch_dependency_deadline")
		}
		e := d.dependencies()
		if e == nil {
			break
		}
		if e.Error() != "dependencies_waiting" && !observationWaiting(e) {
			return e
		}
		pause()
	}
	var c Identity
	var selected bool
	for {
		if now().Unix() >= deadline {
			return errors.New("launch_observation_deadline")
		}
		e = d.revalidate()
		if observationWaiting(e) {
			pause()
			continue
		}
		if e != nil {
			return e
		}
		effective, re := d.effective(true)
		if observationWaiting(re) {
			pause()
			continue
		}
		if re != nil {
			return re
		}
		if !bytesEqualJSON(effective, d.Rec.Effective) {
			return errors.New("selection_configuration_changed")
		}
		c, e = d.container(d.Rec.Invocation.Service)
		if observationWaiting(e) {
			pause()
			continue
		}
		if e != nil {
			return e
		}
		base := d.Rec.Containers[d.Rec.Invocation.Service]
		if (c.ID != base.ID && c.ID != d.Rec.Pair.Container) || (c.Image != base.Image && c.Image != o.S.Desired.ImageID) || !sameRuntime(base, c) {
			return errors.New("launch_actual_identity_changed")
		}
		e = d.selectedActive(c, *o.S.Desired)
		if observationWaiting(e) {
			pause()
			continue
		}
		if e != nil && !selectionNeedsActivation(e) {
			return e
		}
		selected = e == nil
		break
	}
	if !selected {
		// A normal restart is still a mutation. Journal its issue before up so
		// owner death cannot turn an uncertain result into a second activation.
		if o.S.Current != nil {
			o.S.History = append(o.S.History, o.S.Current)
		}
		o.S.Generation++
		o.S.Current = &Operation{ID: token(), Target: o.S.Desired.Tag, Release: *o.S.Desired, Phase: "activating", Generation: o.S.Generation, Prior: c.ID, PriorImage: c.Image, ActivationIssued: true, Limits: o.S.Limits, Deadline: deadline, Updated: now().Unix()}
		o.trim()
		if e = o.persist(); e != nil {
			return e
		}
		if e = d.selectRelease(*o.S.Desired, o.S.Generation); e != nil {
			return e
		}
		if _, e = d.comp(true, true, "up", "-d", "--no-deps", "--no-build", "--pull", "never", d.Rec.Invocation.Service); e != nil {
			if pe := o.fail("compose_exit_recovery_required"); pe != nil {
				return pe
			}
			return e
		}
		if e = o.phase("verifying", o.S.Limits.Verify); e != nil {
			return e
		}
		return errors.New("launch_verification_pending")
	}
	for {
		if now().Unix() >= deadline {
			return errors.New("launch_health_unverified: read status")
		}
		if _, e = d.observed(*o.S.Desired, "", ""); e == nil {
			return nil
		}
		pause()
	}
}

// The ordinary launcher must make the original runtime available even when
// retrying adoption or waiting for an update already owned by the supervisor.
func (o *Owner) launchPending(now func() time.Time, pause func(), start func() error, wait func() error) (bool, error) {
	c := o.S.Current
	if !c.active() && (c == nil || c.Phase != "failed_resolution" && c.Phase != "failed_download") {
		return false, nil
	}
	pause, finish, e := o.launchStatus(now, pause)
	if e != nil {
		return true, e
	}
	// Reconcile expiry under the caller's owner lock, using the controller's
	// existing classification. Post-activation expiry must remain recovery.
	if c.active() && now().Unix() >= c.Deadline {
		if e = o.step(); e != nil {
			return true, finish(e)
		}
	}
	if c.Phase == "failed_resolution" || c.Phase == "failed_download" {
		if c.ActivationIssued {
			return true, finish(errors.New("confirmed_pre_activation_failure_required"))
		}
		// Startup has its own existing budget. Submit only once the original
		// runtime is available so the new Resolve budget starts at resolution.
		if e = o.D.waitAvailable(now().Unix()+o.S.Limits.Activate, now, pause, start); e == nil {
			e = o.resumeSelection()
		}
	} else if c.active() {
		// Existing work retains its absolute deadline without renewal.
		e = o.D.waitAvailable(c.Deadline, now, pause, start)
	}
	if e = finish(e); e != nil {
		return true, e
	}
	if !o.S.Current.active() {
		return false, nil
	}
	return true, wait()
}

func (o *Owner) resumeSelection() error {
	c := o.S.Current
	if c == nil || c.ActivationIssued || (c.Phase != "failed_download" && c.Phase != "failed_resolution") {
		return errors.New("confirmed_pre_activation_failure_required")
	}
	if !tagPattern.MatchString(c.Target) {
		return errors.New("persisted_target_invalid")
	}
	// Preserve the exact failed target, even if an installer carries an older pin.
	_, e := o.submit(Message{Key: token(), Target: c.Target})
	return e
}
func (o *Owner) refresh(contextName string) error {
	if o.S.Current.active() || o.S.Current != nil && o.S.Current.Phase == "recovery_required" {
		return errors.New("refresh_requires_resolved_operation")
	}
	old := o.D.Rec
	for p, stamp := range old.Sources {
		got, e := ownership(p)
		if e != nil || got != stamp {
			return errors.New("source_ownership_changed")
		}
	}
	// Inspect original context first, then require the same actual owned identities
	// through the explicitly chosen context. Never register unrelated resources.
	for name, before := range old.Containers {
		c, e := o.D.container(name)
		if e != nil {
			return e
		}
		image, id := before.Image, before.ID
		if name == old.Invocation.Service && o.S.Desired != nil {
			image, id = o.S.Desired.ImageID, old.Pair.Container
		}
		if c.ID != id || c.Image != image || !sameRuntime(before, c) {
			return errors.New("refresh_original_identity_changed")
		}
	}
	i := old.Invocation
	i.Files = append([]string(nil), i.Files...)
	i.EnvFiles = append([]string(nil), i.EnvFiles...)
	if contextName != "" {
		i.Context = contextName
	}
	next, e := registerExisting(o.Root, i, o.D.R, old, o.S.Desired)
	if e != nil {
		return e
	}
	next.Installation, next.Pair, next.Integrated, next.AdoptionTarget, next.Mailbox = old.Installation, old.Pair, old.Integrated, old.AdoptionTarget, old.Mailbox
	if e = save(filepath.Join(o.Root, "registration.json"), next); e != nil {
		return e
	}
	*old = *next
	return nil
}

func (o *Owner) waitObservation() error {
	c := o.S.Current
	if c.Code == "daemon_observation_waiting" {
		return nil
	}
	c.Code = "daemon_observation_waiting"
	c.Samples = 0
	return o.persist()
}
