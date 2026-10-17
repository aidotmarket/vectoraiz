package main

import (
	"bytes"
	"context"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"sync"
	"time"
)

func itoa(n int) string { return strconv.Itoa(n) }

var allowNativeExecution = true // Isolated tests close this guard before any test.
// Output is bounded while draining; Docker errors never expose resolved secrets.
type boundedBuffer struct {
	mu       sync.Mutex
	b        bytes.Buffer
	max      int
	overflow bool
}

func (b *boundedBuffer) Write(p []byte) (int, error) {
	b.mu.Lock()
	defer b.mu.Unlock()
	n := len(p)
	left := b.max - b.b.Len()
	if n > left {
		b.overflow = true
		p = p[:left]
	}
	b.b.Write(p)
	return n, nil
}

type Command struct {
	ID            string
	Args, Env     []string
	Cwd           string
	Input         []byte
	Deadline      int64
	Started, Done bool
	Code          string
	Output        []byte
	Max           int
	ProcessGroup  int // Persisted helper identity before any external command starts.
}
type Runner interface {
	Run(args []string, input []byte, deadline int64, max int, mutate bool) ([]byte, error)
}

// Start the detached helper, then reap it exactly once while this owner lives.
// Waiting is asynchronous: polling/deadlines and durable JSON remain the outcome
// authority. Wait does not cancel a helper when the polling owner exits; the
// helper retains its own process group/job and finite command deadline.
type helperProcess interface {
	Start() error
	Wait() error
}

func startHelper(c helperProcess) error {
	if e := c.Start(); e != nil {
		return e
	}
	go func() { _ = c.Wait() }()
	return nil
}

type Processes struct {
	Root, Cwd  string
	Env        []string
	GroupAlive func(int) bool // Tests inject observation without native adapters.
}

func (p *Processes) groupAlive(pid int) bool {
	if p.GroupAlive != nil {
		return p.GroupAlive(pid)
	}
	if !allowNativeExecution {
		return true
	}
	return commandGroupAlive(pid)
}

func execute(r *Command) error {
	if !allowNativeExecution {
		return errors.New("ISOLATION_GUARD: native process denied")
	}
	if time.Now().Unix() >= r.Deadline {
		return errors.New("command_deadline")
	}
	ctx, cancel := context.WithDeadline(context.Background(), time.Unix(r.Deadline, 0))
	defer cancel()
	c := exec.CommandContext(ctx, r.Args[0], r.Args[1:]...)
	c.Dir = r.Cwd
	c.Env = r.Env
	c.Stdin = bytes.NewReader(r.Input)
	if r.ProcessGroup == 0 {
		childGroup(c)
	}
	c.Cancel = func() error { killCommand(c, r.ProcessGroup); return nil }
	c.WaitDelay = 2 * time.Second
	out := &boundedBuffer{max: r.Max}
	errout := &boundedBuffer{max: 32768}
	c.Stdout = out
	c.Stderr = errout
	e := c.Run()
	r.Output = out.b.Bytes()
	if ctx.Err() != nil {
		return errors.New("command_deadline")
	}
	if out.overflow {
		return errors.New("command_output_limit")
	}
	if e != nil {
		return errors.New("command_exit")
	}
	return nil
}
func (p *Processes) Run(args []string, input []byte, deadline int64, max int, mutate bool) ([]byte, error) {
	if !allowNativeExecution {
		return nil, errors.New("ISOLATION_GUARD: native process denied")
	}
	r := Command{ID: token(), Args: args, Env: p.Env, Cwd: p.Cwd, Input: input, Deadline: deadline, Max: max}
	if !mutate {
		e := execute(&r)
		return r.Output, e
	}
	// Separate, deadline-bound command owner survives the polling owner. Its OS lock
	// and durable result are reconciled before any subsequent mutation. Uncertain
	// handoff is never re-executed automatically.
	path := filepath.Join(p.Root, "child.json")
	if e := p.Reconcile(); e != nil {
		return nil, e
	}
	if e := save(path, &r); e != nil {
		return nil, e
	}
	exe, e := os.Executable()
	if e != nil {
		return nil, e
	}
	c := exec.Command(exe, "command", p.Root, r.ID)
	childGroup(c)
	if e = startHelper(c); e != nil {
		return nil, errors.New("child_handoff_unknown")
	}
	for {
		var got Command
		if e = load(path, &got); e != nil {
			return nil, e
		}
		if got.ID != r.ID {
			return nil, errors.New("child_identity")
		}
		if got.Done {
			if got.Code != "" {
				return got.Output, errors.New(got.Code)
			}
			return got.Output, nil
		}
		if time.Now().Unix() > deadline+5 {
			return nil, errors.New("child_may_be_running")
		}
		time.Sleep(100 * time.Millisecond)
	}
}
func (p *Processes) Reconcile() error {
	var r Command
	e := load(filepath.Join(p.Root, "child.json"), &r)
	if os.IsNotExist(e) {
		return nil
	}
	if e != nil {
		return e
	}
	if r.Done {
		return nil
	}
	// A surviving helper owns its finite budget. A missing completion remains a
	// recovery state even after deadline: no guess about daemon-side mutation.
	for time.Now().Unix() <= r.Deadline+5 {
		if e = load(filepath.Join(p.Root, "child.json"), &r); e != nil {
			return e
		}
		if r.Done {
			return nil
		}
		time.Sleep(100 * time.Millisecond)
	}
	// A dead helper can leave an unknown outcome, rather than a permanent dead
	// end. Its process group (Unix) or kill-on-close job (Windows) must also be
	// gone. Never kill an unverified/reused identity or replay this command.
	guard, le := lock(filepath.Join(p.Root, "child.lock"))
	if le == nil {
		defer unlock(guard)
		if e = load(filepath.Join(p.Root, "child.json"), &r); e != nil {
			return e
		}
		if r.Done {
			return nil
		}
		if !r.Started || r.ProcessGroup > 0 && !p.groupAlive(r.ProcessGroup) {
			r.Done, r.Code = true, "child_outcome_unknown"
			r.Args, r.Env, r.Input = nil, nil, nil
			if e = save(filepath.Join(p.Root, "child.json"), &r); e != nil {
				return e
			}
			return errors.New("child_outcome_unknown")
		}
	}
	return errors.New("child_may_be_running")
}
func commandMain(root, id string) error {
	f, e := lock(filepath.Join(root, "child.lock"))
	if e != nil {
		return e
	}
	defer unlock(f)
	path := filepath.Join(root, "child.json")
	var r Command
	if e = load(path, &r); e != nil {
		return e
	}
	if r.ID != id || r.Started || r.Done {
		return errors.New("child_identity")
	}
	r.Started = true
	r.ProcessGroup = os.Getpid()
	if e = save(path, &r); e != nil {
		return e
	}
	if e = helperJob(); e != nil {
		r.Code = "child_supervision_unavailable"
		r.Done = true
		r.Args, r.Env, r.Input = nil, nil, nil
		return save(path, &r)
	}
	if e = execute(&r); e != nil {
		r.Code = e.Error()
	}
	r.Done = true
	// Wipe arguments/environment/input from completed records (may hold credentials).
	r.Args = nil
	r.Env = nil
	r.Input = nil
	return save(path, &r)
}
