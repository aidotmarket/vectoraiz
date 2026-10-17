package main

import (
	"archive/tar"
	"bytes"
	"crypto/hmac"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"io"
	"path"
	"regexp"
	"strconv"
	"strings"
	"time"
)

const mailbox = "/run/vectoraiz-owner-mailbox"

var mailboxCandidates = []string{mailbox, "/var/lib/vectoraiz-owner-mailbox", "/opt/vectoraiz-owner-mailbox"}

const repository = "ghcr.io/aidotmarket/vectoraiz"

var tagPattern = regexp.MustCompile(`^v?[0-9]+\.[0-9]+\.[0-9]+(-rc\.[0-9]+)?$`)
var keyPattern = regexp.MustCompile(`^[a-zA-Z0-9_-]{16,128}$`)

func forwardTag(from, to string) bool {
	parse := func(s string) [5]int {
		var v [5]int
		s = strings.TrimPrefix(s, "v")
		parts := strings.SplitN(s, "-rc.", 2)
		for n, x := range strings.Split(parts[0], ".") {
			if n < 3 {
				v[n], _ = strconv.Atoi(x)
			}
		}
		v[3] = 1
		if len(parts) == 2 {
			v[3] = 0
			v[4], _ = strconv.Atoi(parts[1])
		}
		return v
	}
	a, b := parse(from), parse(to)
	for n := range a {
		if a[n] != b[n] {
			return b[n] > a[n]
		}
	}
	return true
}

type Pair struct {
	Installation string `json:"installation"`
	Epoch        string `json:"epoch"`
	Container    string `json:"container"`
	Generation   int    `json:"generation"`
	Key          string `json:"key"`
}

// Payload is a serialized JSON string, so both languages MAC identical bytes.
type Envelope struct {
	Payload string `json:"payload"`
	MAC     string `json:"mac"`
}
type Message struct {
	Kind         string `json:"kind"`
	Installation string `json:"installation"`
	Epoch        string `json:"epoch"`
	Container    string `json:"container"`
	Generation   int    `json:"generation"`
	Key          string `json:"key"`
	Target       string `json:"target"`
	Principal    string `json:"principal"`
	Scope        string `json:"scope"`
	Nonce        string `json:"nonce"`
	Expires      int64  `json:"expires"`
}

func sign(key, kind string, payload any) ([]byte, error) {
	b, e := json.Marshal(payload)
	if e != nil {
		return nil, e
	}
	m := hmac.New(sha256.New, []byte(key))
	m.Write([]byte(kind + "\n"))
	m.Write(b)
	return json.Marshal(Envelope{string(b), hex.EncodeToString(m.Sum(nil))})
}
func verify(key, kind string, b []byte, v any) error {
	if len(b) > 32768 {
		return errors.New("envelope_size")
	}
	var env Envelope
	if e := strictJSON(b, &env); e != nil {
		return e
	}
	sig, e := hex.DecodeString(env.MAC)
	if e != nil {
		return e
	}
	m := hmac.New(sha256.New, []byte(key))
	m.Write([]byte(kind + "\n"))
	m.Write([]byte(env.Payload))
	if !hmac.Equal(sig, m.Sum(nil)) {
		return errors.New("invalid_mac")
	}
	return strictJSON([]byte(env.Payload), v)
}
func strictJSON(b []byte, v any) error {
	d := json.NewDecoder(bytes.NewReader(b))
	d.DisallowUnknownFields()
	if e := d.Decode(v); e != nil {
		return e
	}
	if d.Decode(new(any)) != io.EOF {
		return errors.New("json_trailing")
	}
	return nil
}
func request(pair Pair, b []byte, now int64) (Message, error) {
	var m Message
	if len(b) > 4096 {
		return m, errors.New("request_size")
	}
	e := verify(pair.Key, "request", b, &m)
	if e != nil {
		return m, e
	}
	if m.Kind != "submit" || m.Installation != pair.Installation || m.Epoch != pair.Epoch || m.Container != pair.Container || m.Generation != pair.Generation || m.Scope != "update:submit" || m.Principal == "" || len(m.Principal) > 256 || !keyPattern.MatchString(m.Key) || !tagPattern.MatchString(m.Target) || !keyPattern.MatchString(m.Nonce) {
		return m, errors.New("request_identity")
	}
	if m.Expires < now || m.Expires > now+120 {
		return m, errors.New("request_expired")
	}
	return m, nil
}

// Parse only a single fixed regular member. Never extract container archives.
func unpack(b []byte, name string, max int) ([]byte, error) {
	if len(b) > 65536 || len(b) < 1536 || len(b)%512 != 0 {
		return nil, errors.New("archive_size")
	}
	// Reject extension records before tar.Reader can transparently consume them.
	if b[156] != tar.TypeReg && b[156] != tar.TypeRegA {
		return nil, errors.New("archive_type")
	}
	tr := tar.NewReader(bytes.NewReader(b))
	h, e := tr.Next()
	if e != nil {
		return nil, e
	}
	if h.Name != name || path.Base(h.Name) != h.Name || h.Size < 0 || h.Size > int64(max) || h.Linkname != "" || h.Typeflag != tar.TypeReg && h.Typeflag != tar.TypeRegA {
		return nil, errors.New("archive_member")
	}
	data, e := io.ReadAll(io.LimitReader(tr, int64(max)+1))
	if e != nil || int64(len(data)) != h.Size {
		return nil, errors.New("archive_truncated")
	}
	end := 512 + ((int(h.Size)+511)/512)*512
	if len(b) < end+1024 {
		return nil, errors.New("archive_truncated")
	}
	for _, v := range b[end:] {
		if v != 0 {
			return nil, errors.New("archive_extra")
		}
	}
	if _, e = tr.Next(); e != io.EOF {
		return nil, errors.New("archive_extra")
	}
	return data, nil
}
func pack(name string, b []byte) ([]byte, error) {
	if len(b) > 32768 {
		return nil, errors.New("mailbox_size")
	}
	var out bytes.Buffer
	w := tar.NewWriter(&out)
	if e := w.WriteHeader(&tar.Header{Name: name, Mode: 0600, Size: int64(len(b)), Typeflag: tar.TypeReg, Uid: 0, Gid: 0, ModTime: time.Unix(0, 0), Format: tar.FormatUSTAR}); e != nil {
		return nil, e
	}
	if _, e := w.Write(b); e != nil {
		return nil, e
	}
	if e := w.Close(); e != nil {
		return nil, e
	}
	return out.Bytes(), nil
}
