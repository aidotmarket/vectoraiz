//go:build windows

package main

import (
	"encoding/base64"
	"encoding/binary"
	"errors"
	"os"
	"os/exec"
	"syscall"
	"time"
	"unicode/utf16"
	"unsafe"
)

var kernel = syscall.NewLazyDLL("kernel32.dll")
var lockFileEx = kernel.NewProc("LockFileEx")
var moveFileEx = kernel.NewProc("MoveFileExW")

// Exclusive OS handle lock; process termination releases it. No PID-file lock.
func lock(p string) (*os.File, error) {
	f, e := os.OpenFile(p, os.O_CREATE|os.O_RDWR, 0600)
	if e != nil {
		return nil, e
	}
	var ov syscall.Overlapped
	r, _, _ := lockFileEx.Call(f.Fd(), 3, 0, 1, 0, uintptr(unsafe.Pointer(&ov)))
	if r == 0 {
		f.Close()
		return nil, errors.New("installation_busy")
	}
	return f, nil
}
func unlock(f *os.File) { f.Close() }
func replaceFile(a, b string) error {
	ap, e := syscall.UTF16PtrFromString(a)
	if e != nil {
		return e
	}
	bp, e := syscall.UTF16PtrFromString(b)
	if e != nil {
		return e
	}
	r, _, e := moveFileEx.Call(uintptr(unsafe.Pointer(ap)), uintptr(unsafe.Pointer(bp)), 9)
	if r == 0 {
		return e
	}
	return nil
}
func syncDir(string) error { return nil } // MoveFileEx WRITE_THROUGH + file FlushFileBuffers.
func acl(p string, private bool) (string, error) {
	// Fixed script, path passed as private environment data, never interpolated.
	script := `$ErrorActionPreference='Stop'; $a=Get-Acl -LiteralPath $env:VECTORAIZ_OWNER_ACL_PATH; $me=[System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value; $owner=$a.GetOwner([System.Security.Principal.SecurityIdentifier]).Value; if($owner -ne $me -and $owner -ne 'S-1-5-18' -and $owner -ne 'S-1-5-32-544'){exit 2}; foreach($r in $a.Access){$sid=$r.IdentityReference.Translate([System.Security.Principal.SecurityIdentifier]).Value; if($r.AccessControlType -eq 'Allow' -and $sid -ne $me -and $sid -ne 'S-1-5-18' -and $sid -ne 'S-1-5-32-544'){if($env:VECTORAIZ_OWNER_ACL_PRIVATE -eq '1' -or (($r.FileSystemRights.value__ -band 0xD0156) -ne 0)){exit 3}}}; if($env:VECTORAIZ_OWNER_ACL_PRIVATE -eq '1' -and (-not $a.AreAccessRulesProtected -or $owner -ne $me)){exit 4}; [Console]::Write($a.Sddl)`
	words := utf16.Encode([]rune(script))
	raw := make([]byte, len(words)*2)
	for n, w := range words {
		binary.LittleEndian.PutUint16(raw[n*2:], w)
	}
	value := "0"
	if private {
		value = "1"
	}
	r := Command{Args: []string{"powershell.exe", "-NoProfile", "-NonInteractive", "-EncodedCommand", base64.StdEncoding.EncodeToString(raw)}, Env: append(os.Environ(), "VECTORAIZ_OWNER_ACL_PATH="+p, "VECTORAIZ_OWNER_ACL_PRIVATE="+value), Deadline: time.Now().Unix() + 15, Max: 32768}
	if e := execute(&r); e != nil {
		return "", errors.New("acl_unavailable_or_unsafe")
	}
	return string(r.Output), nil
}
func secureDirectory(p string) error {
	s, e := os.Lstat(p)
	if e != nil {
		return e
	}
	if !s.IsDir() || s.Mode()&os.ModeSymlink != 0 {
		return errors.New("private_directory")
	}
	_, e = acl(p, true)
	return e
}
func ownership(p string) (string, error) { return acl(p, false) }
func childGroup(c *exec.Cmd)             { c.SysProcAttr = &syscall.SysProcAttr{CreationFlags: 0x00000200} }
func killTree(c *exec.Cmd) {
	if !allowNativeExecution {
		return
	}
	if c.Process != nil { // taskkill targets only the tracked child tree.
		exec.Command("taskkill.exe", "/PID", itoa(c.Process.Pid), "/T", "/F").Run()
	}
}
func killCommand(c *exec.Cmd, group int) { killTree(c) }
func commandGroupAlive(pid int) bool {
	h, e := syscall.OpenProcess(0x00100000, false, uint32(pid)) // SYNCHRONIZE
	if e != nil {
		return e != syscall.Errno(87) // ERROR_INVALID_PARAMETER: PID no longer exists.
	}
	defer syscall.CloseHandle(h)
	result, e := syscall.WaitForSingleObject(h, 0)
	return e != nil || result != 0
}
func helperJob() error {
	if !allowNativeExecution {
		return errors.New("ISOLATION_GUARD: native job denied")
	}
	// The helper owns this handle for its complete lifetime. On normal or abrupt
	// exit Windows kills remaining descendants before orphan reconciliation.
	var limits struct {
		Basic struct {
			ProcessTime, JobTime   int64
			Flags                  uint32
			MinWorking, MaxWorking uintptr
			Active                 uint32
			Affinity               uintptr
			Priority, Scheduling   uint32
		}
		IO     [6]uint64
		Memory [4]uintptr
	}
	limits.Basic.Flags = 0x2000 // JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
	h, _, e := kernel.NewProc("CreateJobObjectW").Call(0, 0)
	if h == 0 {
		return e
	}
	ok, _, e := kernel.NewProc("SetInformationJobObject").Call(h, 9, uintptr(unsafe.Pointer(&limits)), unsafe.Sizeof(limits))
	if ok == 0 {
		syscall.CloseHandle(syscall.Handle(h))
		return e
	}
	self, _ := syscall.GetCurrentProcess()
	ok, _, e = kernel.NewProc("AssignProcessToJobObject").Call(h, uintptr(self))
	if ok == 0 {
		syscall.CloseHandle(syscall.Handle(h))
		return e
	}
	// Deliberately retained until process exit, after the durable result is saved.
	return nil
}
