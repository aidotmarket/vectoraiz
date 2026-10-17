package main

import (
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"
)

type listFlag []string

var packageVersion = "dev" // Set from the verified release tag by owner-assets.

func (v *listFlag) String() string     { return strings.Join(*v, ",") }
func (v *listFlag) Set(s string) error { *v = append(*v, s); return nil }
func runMain(args []string) error {
	if len(args) == 1 && args[0] == "version" {
		fmt.Println(packageVersion)
		return nil
	}
	if len(args) == 3 && args[0] == "command" {
		return commandMain(args[1], args[2])
	}
	if len(args) < 1 {
		return errors.New("usage: vectoraiz-owner setup|serve|launch|status|reconcile --root PATH")
	}
	command := args[0]
	fs := flag.NewFlagSet(command, flag.ContinueOnError)
	root := fs.String("root", ".vectoraiz-owner", "private installation state")
	target := fs.String("target", "", "exact adoption release tag")
	contextName := fs.String("context", "", "original Docker context")
	project := fs.String("project", "", "original Compose project")
	projectDir := fs.String("project-directory", "", "original project directory")
	cwd := fs.String("cwd", "", "original invocation working directory")
	channel := fs.String("channel", "direct", "original presentation channel")
	openBrowser := fs.Bool("open", false, "open configured application after verified launch")
	appURL := fs.String("url", "", "existing browser URL, including remote routing")
	var files, envFiles listFlag
	fs.Var(&files, "file", "ordered original Compose file (repeatable)")
	fs.Var(&envFiles, "env-file", "ordered original env file (repeatable)")
	if e := fs.Parse(args[1:]); e != nil {
		return e
	}
	*root = absolute(".", *root)
	if command == "status" { // No lock, daemon, application or owner required.
		var s Snapshot
		if e := load(filepath.Join(*root, "status.json"), &s); e != nil {
			return e
		}
		out := map[string]any{"snapshot": s, "stale": time.Now().Unix()-s.Heartbeat > 15}
		return json.NewEncoder(os.Stdout).Encode(out)
	}
	if command == "wait" { // Bounded adoption UI, never a timer-based success signal.
		until := time.Now().Add(3 * time.Hour)
		last := ""
		for time.Now().Before(until) {
			var s Snapshot
			if e := load(filepath.Join(*root, "status.json"), &s); e != nil {
				return e
			}
			if s.Current != nil {
				c := s.Current
				if c.Phase != last {
					fmt.Println("Owner phase:", c.Phase)
					last = c.Phase
				}
				if c.Phase == "succeeded" && time.Now().Unix()-s.Heartbeat <= 15 {
					return nil
				}
				if c.Phase == "recovery_required" || c.Phase == "failed_download" || c.Phase == "failed_resolution" {
					return errors.New(c.Phase + ": " + c.Code + "; read status.json; configuration and data retained")
				}
				if time.Now().Unix() > c.Deadline+30 {
					return errors.New("owner_outcome_unknown: read status.json; do not repeat adoption")
				}
			}
			time.Sleep(time.Second)
		}
		return errors.New("owner_outcome_unknown: adoption watchdog expired")
	}
	if e := privateDir(*root); e != nil {
		return e
	}
	lockName := "owner.lock"
	if command == "serve" {
		lockName = "daemon.lock"
	}
	f, e := lock(filepath.Join(*root, lockName))
	if e != nil {
		if command == "launch" {
			return errors.New("owner_busy: use status; an accepted update continues independently")
		}
		return e
	}
	defer unlock(f)
	var rec Record
	if command == "setup" {
		if e = load(filepath.Join(*root, "registration.json"), &rec); e == nil {
			return errors.New("already_registered: use launch or status; original context retained")
		}
		if !os.IsNotExist(e) {
			return e
		}
		if !tagPattern.MatchString(*target) {
			return errors.New("exact_adoption_target_required")
		}
		if len(files) == 0 {
			return errors.New("original_invocation_required: repeat --file and --env-file in original order")
		}
		docker, e := exec.LookPath("docker")
		if e != nil {
			return e
		}
		if *cwd == "" {
			*cwd, _ = os.Getwd()
		}
		env := os.Environ()
		if *contextName == "" && os.Getenv("DOCKER_HOST") == "" {
			r := Command{Args: []string{docker, "context", "show"}, Env: env, Cwd: *cwd, Deadline: time.Now().Unix() + 15, Max: 32768}
			if e = execute(&r); e != nil {
				return e
			}
			*contextName = strings.TrimSpace(string(r.Output))
		}
		p := &Processes{Root: *root, Cwd: *cwd, Env: env}
		r, e := register(*root, Invocation{Docker: docker, Cwd: *cwd, ProjectDirectory: *projectDir, Project: *project, Context: *contextName, Files: files, EnvFiles: envFiles, Environment: env, DefaultEnv: len(envFiles) == 0, Service: "vectoraiz", Channel: *channel}, p)
		if e != nil {
			return e
		}
		rec = *r
        rec.AdoptionTarget = *target
		if *appURL != "" && !strings.HasPrefix(*appURL, "http://") && !strings.HasPrefix(*appURL, "https://") {
			return errors.New("browser_url_invalid")
		}
		if e = save(filepath.Join(*root, "browser.json"), *appURL); e != nil {
			return e
		}
		if e = save(filepath.Join(*root, "registration.json"), &rec); e != nil {
			return e
		}
        return completeSetup(*root, &rec, p)

	}
	if e = load(filepath.Join(*root, "registration.json"), &rec); e != nil {
		return errors.New("adoption_required")
	}
	p := &Processes{Root: *root, Cwd: rec.Invocation.Cwd, Env: rec.Invocation.Environment}
    if !rec.Integrated {
        if command == "launch" { return completeSetup(*root, &rec, p) }
        return errors.New("launcher_supervisor_not_integrated")
    }
	d := &Docker{R: p, Rec: &rec, Root: *root}
	o, e := openOwner(*root, d)
	if e != nil {
		return e
	}
	if command != "serve" {
		if e = p.Reconcile(); e != nil {
			o.Reason = e.Error()
			if o.S.Current.active() {
				o.fail(e.Error())
			}
			o.publish()
			return e
		}
	}
	switch command {
	case "serve":
		for {
			guard, le := lock(filepath.Join(*root, "owner.lock"))
			if le != nil {
				time.Sleep(time.Second)
				continue
			}
			// Host repair/launch and this supervisor share the same durable state.
			// Reload under the OS lock so a long-lived owner cannot overwrite a
			// recovery operation committed by another authorized CLI process.
			if e = load(filepath.Join(*root, "registration.json"), &rec); e != nil {
				unlock(guard)
				return e
			}
			o, e = openOwner(*root, d)
			if e != nil {
				unlock(guard)
				return e
			}
			if e = p.Reconcile(); e != nil {
				o.Reason = e.Error()
				if o.S.Current.active() {
					o.fail(e.Error())
				}
				o.publish()
				unlock(guard)
				time.Sleep(2 * time.Second)
				continue
			}
			if e = o.tick(); e != nil { // Preserve redacted host state during daemon/app outages.
				o.Reason = "owner_reconcile_unknown"
				o.publish()
			}
			unlock(guard)
			time.Sleep(2 * time.Second)
		}
	case "reconcile": // Read/inspect recovery; never starts another activation.
		if e = d.revalidate(); e != nil {
			return e
		}
		if o.S.Current != nil {
			c, ce := d.container(rec.Invocation.Service)
			if ce == nil {
				o.S.Current.ObservedImage = c.Image
				o.S.Current.ObservedContainer = c.ID
			}
			if o.S.Current.Phase == "recovery_required" {
				o.S.Current.Code = "operator_forward_repair_required"
				o.persist()
			}
		}
		o.Reason = "reconciled_status_only"
		return o.publish()
	case "repair":
		if !tagPattern.MatchString(*target) {
			return errors.New("exact_forward_repair_target_required")
		}
		if o.S.Current == nil || o.S.Current.Phase != "recovery_required" {
			return errors.New("no_recovery_operation")
		}
		if !forwardTag(o.S.Current.Target, *target) {
			return errors.New("automatic_or_backward_downgrade_refused")
		}
		if e = d.revalidate(); e != nil {
			return e
		}
		c, e := d.container(rec.Invocation.Service)
		if e != nil {
			return e
		}
		if !sameRuntime(rec.Containers[rec.Invocation.Service], c) || c.Image != rec.Containers[rec.Invocation.Service].Image && o.S.Desired != nil && c.Image != o.S.Desired.ImageID {
			return errors.New("actual_state_unresolved")
		}
		o.S.Current.ObservedImage = c.Image
		o.S.Current.ObservedContainer = c.ID
		o.S.History = append(o.S.History, o.S.Current)
		o.S.Current = nil
		_, e = o.submit(Message{Key: token(), Target: *target})
		return e
	case "retry":
		if !tagPattern.MatchString(*target)||o.S.Current==nil||(o.S.Current.Phase!="failed_download"&&o.S.Current.Phase!="failed_resolution"){return errors.New("retry_requires_a_confirmed_pre_activation_failure_and_exact_target")}
		if e=d.revalidate();e!=nil{return e};if _,e=o.submit(Message{Key:token(),Target:*target});e!=nil{return e};fmt.Println("Retry durably accepted; use wait/status for the actual outcome.");return startSupervisor(*root,rec.Installation)
	case "launch":
        // Idempotently recover a supervisor handoff lost during one-time setup.
        if e=startSupervisor(*root,rec.Installation);e!=nil{return errors.New("owner_supervisor_unavailable: read status.json")}
		if o.S.Current.active() || o.S.Current != nil && o.S.Current.Phase == "recovery_required" {
			return errors.New("update_or_recovery_in_progress: read status; do not repeat activation")
		}
		if o.S.Desired == nil || o.S.LastVerified == nil {
			return errors.New("no_verified_selection")
		}
		if e = d.revalidate(); e != nil {
			return e
		}
		if e = d.selectRelease(*o.S.Desired, o.S.Generation); e != nil {
			return e
		}
		deadline := time.Now().Unix() + o.S.Limits.Activate
		d.Until = deadline
		for {
			e = d.dependencies()
			if e == nil {
				break
			}
			if e.Error() != "dependencies_waiting" || time.Now().Unix() >= deadline {
				return e
			}
			time.Sleep(time.Second)
		}
		effective, e := d.effective(true)
		if e != nil || !bytesEqualJSON(effective, rec.Effective) {
			return errors.New("selection_configuration_changed")
		}
		if _, e = d.comp(true, true, "up", "-d", "--no-deps", "--no-build", "--pull", "never", rec.Invocation.Service); e != nil {
			return e
		}
		for {
			_, e = d.observed(*o.S.Desired, "")
			if e == nil {
				break
			}
			if time.Now().Unix() >= deadline {
				return errors.New("launch_health_unverified: read status")
			}
			time.Sleep(2 * time.Second)
		}
		if *openBrowser {
			var u string
			if e = load(filepath.Join(*root, "browser.json"), &u); e != nil {
				return e
			}
			return browser(u)
		}
		return nil
	default:
		return errors.New("unknown_command")
	}
}
func completeSetup(root string,rec *Record,p *Processes)error {
    if !tagPattern.MatchString(rec.AdoptionTarget){return errors.New("adoption_target_unresolved")}
    d:=&Docker{R:p,Rec:rec,Root:root}
    if e:=d.revalidate();e!=nil{return e}
    o,e:=openOwner(root,d);if e!=nil{return e}
    o.Reason="adoption_in_progress"
    // A durable host status must exist before any launcher or supervisor change.
    b,e:=json.Marshal(o.snapshot());if e!=nil{return e}
    if e=atomicFile(filepath.Join(root,"status.json"),b);e!=nil{return e}
    if e=integrate(root,rec.Installation);e!=nil{return e}
    if o.S.Current==nil {if _,e=o.submit(Message{Key:token(),Target:rec.AdoptionTarget});e!=nil{return e}}
    rec.Integrated=true
    if e=save(filepath.Join(root,"registration.json"),rec);e!=nil{return e}
    return startSupervisor(root,rec.Installation)
}

func main() {
	if e := runMain(os.Args[1:]); e != nil {
		fmt.Fprintln(os.Stderr, e.Error())
		os.Exit(1)
	}
}
