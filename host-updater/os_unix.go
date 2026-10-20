//go:build darwin || linux

package main

import (
	"errors"
	"io"
	"os"
	"os/exec"
	"strconv"
	"syscall"
	"time"
)

func lock(path string) (*os.File, error) {
	f, e := os.OpenFile(path, os.O_CREATE|os.O_RDWR, 0600)
	if e != nil {
		return nil, e
	}
	if e = syscall.Flock(int(f.Fd()), syscall.LOCK_EX|syscall.LOCK_NB); e != nil {
		f.Close()
		return nil, errors.New("installation_busy")
	}
	return f, nil
}
func unlock(f *os.File)             { syscall.Flock(int(f.Fd()), syscall.LOCK_UN); f.Close() }
func replaceFile(a, b string) error { return os.Rename(a, b) }
func syncDir(p string) error {
	f, e := os.Open(p)
	if e != nil {
		return e
	}
	defer f.Close()
	return f.Sync()
}
func secureDirectory(p string) error {
	s, e := os.Lstat(p)
	if e != nil {
		return e
	}
	if !s.IsDir() || s.Mode().Perm()&0077 != 0 {
		return errors.New("private_directory_permissions")
	}
	st := s.Sys().(*syscall.Stat_t)
	if int(st.Uid) != os.Getuid() {
		return errors.New("wrong_owner")
	}
	return nil
}
func ownership(p string) (string, error) {
	s, e := os.Lstat(p)
	if e != nil {
		return "", e
	}
	if s.Mode()&os.ModeSymlink != 0 {
		return "", errors.New("source_symlink")
	}
	st := s.Sys().(*syscall.Stat_t)
	if int(st.Uid) != os.Getuid() && st.Uid != 0 {
		return "", errors.New("source_wrong_owner")
	}
	if s.Mode().Perm()&0022 != 0 {
		return "", errors.New("source_writable_by_others")
	}
	return strconv.FormatUint(uint64(st.Uid), 10) + ":" + strconv.FormatUint(uint64(st.Gid), 10) + ":" + s.Mode().String(), nil
}
func childGroup(c *exec.Cmd) { c.SysProcAttr = &syscall.SysProcAttr{Setpgid: true} }
func killTree(c *exec.Cmd) {
	if c.Process != nil {
		syscall.Kill(-c.Process.Pid, syscall.SIGKILL)
	}
}

// A second member pins the helper's live group, so its kill never targets a
// persisted/reused identity. Its stdin is a private pipe: only the helper holds
// the write end (CLOEXEC), independent of the polling owner. No daemon/listener.
func superviseHelper(deadline int64) error {
	if !allowNativeExecution {
		return errors.New("ISOLATION_GUARD: native supervision denied")
	}
	exe, e := os.Executable()
	if e != nil {
		return e
	}
	lifeRead, lifeWrite, e := os.Pipe()
	if e != nil {
		return e
	}
	readyRead, readyWrite, e := os.Pipe()
	if e != nil {
		lifeRead.Close()
		lifeWrite.Close()
		return e
	}
	c := exec.Command(exe, "watchdog", strconv.FormatInt(deadline, 10), itoa(os.Getpid()))
	c.SysProcAttr = &syscall.SysProcAttr{Setpgid: true, Pgid: os.Getpid()}
	c.Stdin, c.Stdout = lifeRead, readyWrite
	e = startHelper(c)
	lifeRead.Close()
	readyWrite.Close()
	if e != nil {
		lifeWrite.Close()
		readyRead.Close()
		return e
	}
	// Handshake is bounded by the existing absolute command budget. Do not start
	// Docker unless the group-pinning watchdog has entered its monitoring loop.
	if e = readyRead.SetReadDeadline(time.Unix(deadline, 0)); e == nil {
		var b [1]byte
		_, e = io.ReadFull(readyRead, b[:])
		if e == nil && b[0] != 1 {
			e = errors.New("watchdog_not_ready")
		}
	}
	readyRead.Close()
	if e != nil {
		lifeWrite.Close()
		return e
	}
	// Keep reachable until process exit, including durable save and unlock.
	// It must not be finalized early or inherited by the external command.
	helperLifetime = lifeWrite
	return nil
}

var helperLifetime *os.File

func watchdogMain(deadline int64, group int) error {
	if !allowNativeExecution {
		return errors.New("ISOLATION_GUARD: native watchdog denied")
	}
	if group <= 1 || syscall.Getpgrp() != group || os.Getppid() != group {
		return errors.New("watchdog_identity")
	}
	ended := make(chan struct{})
	go func() { _, _ = io.Copy(io.Discard, os.Stdin); close(ended) }()
	timer := time.NewTimer(time.Until(time.Unix(deadline, 0)))
	defer timer.Stop()
	if _, e := os.Stdout.Write([]byte{1}); e != nil {
		return e // No readiness -> helper cannot execute an external command.
	}
	watchLifetime(ended, timer.C, func() {
		// The watchdog is itself still a member: the group cannot be reused.
		_ = syscall.Kill(-syscall.Getpgrp(), syscall.SIGKILL)
	})
	return errors.New("watchdog_kill_failed")
}
func commandGroupAlive(pid int) bool {
	e := syscall.Kill(-pid, 0)
	return e != syscall.ESRCH
}
func killCommand(c *exec.Cmd, group int) {
	if group > 0 {
		// The tracked helper and its children share this group. A deadline kill
		// can also end the helper; the durable unknown outcome is reconciled.
		syscall.Kill(-group, syscall.SIGKILL)
		return
	}
	killTree(c)
}
