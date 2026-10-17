//go:build darwin || linux

package main

import (
	"errors"
	"os"
	"os/exec"
	"strconv"
	"syscall"
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
