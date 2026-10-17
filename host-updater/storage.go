package main

import (
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
)

func token() string {
	var b [24]byte
	if _, e := rand.Read(b[:]); e != nil {
		panic(e)
	}
	return hex.EncodeToString(b[:])
}
func atomicFile(path string, b []byte) error {
	f, e := os.CreateTemp(filepath.Dir(path), ".pending-")
	if e != nil {
		return e
	}
	name := f.Name()
	defer os.Remove(name)
	if e = f.Chmod(0600); e == nil {
		_, e = f.Write(b)
	}
	if e == nil {
		e = f.Sync()
	}
	ce := f.Close()
	if e == nil {
		e = ce
	}
	if e != nil {
		return e
	}
	if e = replaceFile(name, path); e != nil {
		return e
	}
	return syncDir(filepath.Dir(path))
}
func save(path string, v any) error {
	b, e := json.Marshal(v)
	if e != nil {
		return e
	}
	return atomicFile(path, b)
}
func load(path string, v any) error {
	b, e := os.ReadFile(path)
	if e != nil {
		return e
	}
	if len(b) > 8<<20 {
		return errors.New("state_size")
	}
	return json.Unmarshal(b, v)
}
func privateDir(path string) error {
	if e := os.MkdirAll(path, 0700); e != nil {
		return e
	}
	return secureDirectory(path)
}
