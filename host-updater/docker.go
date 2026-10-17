package main

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"reflect"
	"sort"
	"strings"
	"time"
)

type Invocation struct {
	Docker, Cwd, ProjectDirectory, Project, Context, Service, Channel string
	Files, EnvFiles, Environment                                      []string
	DefaultEnv                                                        bool
}
type Identity struct {
	ID     string `json:"Id"`
	Image  string
	Config struct {
		Image  string
		Env    []string
		User   string
		Labels map[string]string
	}
	HostConfig struct {
		ReadonlyRootfs bool
		RestartPolicy  any
		Tmpfs          map[string]string
	}
	Mounts []map[string]any
	State  struct {
		Running bool
		Status  string
		Health  struct{ Status string }
	}
}
type Record struct {
	Invocation   Invocation
	Installation string
	Sources      map[string]string
	Credentials  map[string]string
	Original     json.RawMessage
	Effective    json.RawMessage
	Endpoint     json.RawMessage
	Containers   map[string]Identity
	Hashes       map[string]string
	Platform     string
	Pair         Pair
	Integrated   bool
	AdoptionTarget string
	Mailbox string
}
type Release struct{ Tag, IndexDigest, Digest, ImageID, Platform string }
type Docker struct {
	R     Runner
	Rec   *Record
	Root  string
	Until int64
}

func (d *Docker) deadline() int64 {
	n := time.Now().Unix() + 30
	if d.Until > 0 && d.Until < n {
		return d.Until
	}
	return n
}
func (d *Docker) args(a ...string) []string {
	prefix := []string{d.Rec.Invocation.Docker}
	if d.Rec.Invocation.Context != "" {
		prefix = append(prefix, "--context", d.Rec.Invocation.Context)
	}
	return append(prefix, a...)
}
func (d *Docker) run(mutate bool, input []byte, max int, a ...string) ([]byte, error) {
	return d.R.Run(d.args(a...), input, d.deadline(), max, true)
}
func (d *Docker) compose(selected bool, a ...string) []string {
	i := d.Rec.Invocation
	p := []string{"compose", "--project-directory", i.ProjectDirectory}
	if i.Project != "" {
		p = append(p, "-p", i.Project)
	}
	for _, f := range i.Files {
		p = append(p, "-f", f)
	}
	for _, f := range i.EnvFiles {
		p = append(p, "--env-file", f)
	}
	if selected {
		p = append(p, "-f", filepath.Join(d.Root, "selection.compose.json"))
	}
	return append(p, a...)
}
func (d *Docker) comp(selected, mutate bool, a ...string) ([]byte, error) {
	return d.run(mutate, nil, 4<<20, d.compose(selected, a...)...)
}
func (d *Docker) inspect(id string) (Identity, error) {
	var got []Identity
	b, e := d.run(false, nil, 2<<20, "inspect", id)
	if e != nil {
		return Identity{}, e
	}
	if e = json.Unmarshal(b, &got); e != nil || len(got) != 1 {
		return Identity{}, errors.New("inspect_identity")
	}
	return got[0], nil
}
func (d *Docker) container(service string) (Identity, error) {
	b, e := d.comp(false, false, "ps", "-a", "-q", service)
	if e != nil {
		return Identity{}, e
	}
	ids := strings.Fields(string(b))
	if len(ids) != 1 {
		return Identity{}, errors.New("ambiguous_or_missing_service")
	}
	got, e := d.inspect(ids[0])
	if e != nil {
		return got, e
	}
	if got.Config.Labels["com.docker.compose.project"] != d.Rec.Invocation.Project || got.Config.Labels["com.docker.compose.service"] != service {
		return got, errors.New("wrong_project_service")
	}
	return got, nil
}
func normalizeConfig(b []byte, service string) ([]byte, error) {
	var config map[string]any
	if e := json.Unmarshal(b, &config); e != nil {
		return nil, e
	}
	services, ok := config["services"].(map[string]any)
	if !ok {
		return nil, errors.New("compose_services")
	}
	s, ok := services[service].(map[string]any)
	if !ok {
		return nil, errors.New("compose_app")
	}
	delete(s, "image")
	if env, ok := s["environment"].(map[string]any); ok {
		delete(env, "VECTORAIZ_VERSION")
		delete(env, "VECTORAIZ_OWNER_MAILBOX")
	}
	return json.Marshal(config)
}
func (d *Docker) effective(selected bool) ([]byte, error) {
	b, e := d.comp(selected, false, "config", "--format", "json")
	if e != nil {
		return nil, e
	}
	return normalizeConfig(b, d.Rec.Invocation.Service)
}
func sameRuntime(a, b Identity) bool {
	return reflect.DeepEqual(a.Mounts, b.Mounts) && reflect.DeepEqual(a.HostConfig.RestartPolicy, b.HostConfig.RestartPolicy) && a.HostConfig.ReadonlyRootfs == b.HostConfig.ReadonlyRootfs && reflect.DeepEqual(a.HostConfig.Tmpfs, b.HostConfig.Tmpfs) && a.Config.User == b.Config.User
}
func (d *Docker) revalidate() error {
	for p, stamp := range d.Rec.Sources {
		s, e := ownership(p)
		if e != nil || s != stamp {
			return errors.New("source_ownership_changed")
		}
	}
	for p, want := range d.Rec.Credentials {
		got, e := credentialFingerprint(p)
		if e != nil || got != want {
			return errors.New("credential_source_changed")
		}
	}
	b, e := d.comp(false, false, "config", "--format", "json")
	if e != nil {
		return e
	}
	if !bytesEqualJSON(b, d.Rec.Original) {
		return errors.New("effective_configuration_changed")
	}
	endpoint, e := d.endpoint()
	if e != nil {
		return e
	}
	if !bytesEqualJSON(endpoint, d.Rec.Endpoint) {
		return errors.New("docker_endpoint_changed")
	}
	return nil
}
func credentialFingerprint(p string) (string, error) {
	b, e := os.ReadFile(p)
	if os.IsNotExist(e) {
		return "absent", nil
	}
	if e != nil {
		return "", e
	}
	var config map[string]any
	if json.Unmarshal(b, &config) != nil {
		return "", errors.New("credential_source_invalid")
	}
	effective := map[string]any{}
	for _, k := range []string{"auths", "credsStore", "credHelpers", "currentContext"} {
		if v, ok := config[k]; ok {
			effective[k] = v
		}
	}
	b, _ = json.Marshal(effective)
	return digest(b), nil
}
func (d *Docker) captureCredentials() error {
	env := map[string]string{}
	for _, s := range d.Rec.Invocation.Environment {
		k, v, ok := strings.Cut(s, "=")
		if ok {
			env[k] = v
		}
	}
	dir := env["DOCKER_CONFIG"]
	if dir == "" {
		home := env["HOME"]
		if home == "" {
			home = env["USERPROFILE"]
		}
		if home == "" {
			return errors.New("original_docker_credential_origin_required")
		}
		dir = filepath.Join(home, ".docker")
	}
	p := absolute(d.Rec.Invocation.Cwd, filepath.Join(dir, "config.json"))
	h, e := credentialFingerprint(p)
	if e != nil {
		return e
	}
	d.Rec.Credentials = map[string]string{p: h}
	if h != "absent" {
		stamp, e := ownership(p)
		if e != nil {
			return e
		}
		d.Rec.Sources[p] = stamp
	}
	return nil
}
func effectiveMetadata(v any) {
	if m, ok := v.(map[string]any); ok {
		for k := range m {
			if strings.HasPrefix(k, "x-") {
				delete(m, k)
			}
		}
		if s, ok := m["services"].(map[string]any); ok {
			for _, v := range s {
				effectiveMetadata(v)
			}
		}
	}
}
func bytesEqualJSON(a, b []byte) bool {
	var av, bv any
	if json.Unmarshal(a, &av) != nil || json.Unmarshal(b, &bv) != nil {
		return false
	}
	effectiveMetadata(av)
	effectiveMetadata(bv)
	return reflect.DeepEqual(av, bv)
}
func (d *Docker) endpoint() ([]byte, error) {
	a := []string{"context", "inspect"}
	if d.Rec.Invocation.Context != "" {
		a = append(a, d.Rec.Invocation.Context)
	}
	b, e := d.run(false, nil, 65536, a...)
	if e != nil {
		return nil, e
	}
	var got []struct {
		Endpoints   any
		TLSMaterial any
	}
	if json.Unmarshal(b, &got) != nil || len(got) != 1 {
		return nil, errors.New("context_endpoint_unresolved")
	}
	return json.Marshal(got)
}
func (d *Docker) copyDeadline() int64 {
	n := time.Now().Unix() + 15
	if d.Until > 0 && d.Until < n {
		return d.Until
	}
	return n
}
func (d *Docker) mailboxPath()string{if d.Rec.Mailbox!=""{for _,p:=range mailboxCandidates{if d.Rec.Mailbox==p{return p}};panic("invalid registered mailbox")};return mailbox}
func (d *Docker) readBox(cid, name string, max int) ([]byte, error) {
	b, e := d.R.Run(d.args("cp", cid+":"+d.mailboxPath()+"/"+name, "-"), nil, d.copyDeadline(), 65536, true)
	if e != nil {
		return nil, e
	}
	return unpack(b, name, max)
}
func (d *Docker) writeBox(cid, name string, b []byte) error {
	a, e := pack(name, b)
	if e != nil {
		return e
	}
	_, e = d.R.Run(d.args("cp", "-", cid+":"+d.mailboxPath()+"/"), a, d.copyDeadline(), 32768, true)
	return e
}
func mailboxPossible(c Identity) error {
	return mailboxAt(c,mailbox)
}
func chooseMailbox(c Identity)(string,error){for _,p:=range mailboxCandidates{if mailboxAt(c,p)==nil{return p,nil}};return "",errors.New("no_writable_nonmounted_mailbox")}
func mailboxAt(c Identity,parent string)error{
	if c.HostConfig.ReadonlyRootfs {
		return errors.New("mailbox_read_only")
	}
	if c.Config.User != "" && c.Config.User != "root" && c.Config.User != "0" && c.Config.User != "0:0" {
		return errors.New("mailbox_user_unsupported")
	}
	for _, m := range c.Mounts {
		p, _ := m["Destination"].(string)
		p = strings.TrimRight(p, "/")
		if p == "" || p == parent || strings.HasPrefix(parent, p+"/") {
			return errors.New("mailbox_mounted")
		}
	}
	for p := range c.HostConfig.Tmpfs {
		p = strings.TrimRight(p, "/")
		if p == "" || p == parent || strings.HasPrefix(parent, p+"/") {
			return errors.New("mailbox_tmpfs")
		}
	}
	return nil
}
func (d *Docker) pair(c Identity, generation int) error {
	if e := mailboxAt(c,d.mailboxPath()); e != nil {
		return e
	}
	p := Pair{Installation: d.Rec.Installation, Epoch: token(), Container: c.ID, Generation: generation, Key: token()}
	// The app must actually read this random challenge and write a signed echo.
	// Successful docker cp alone does not establish a working app writer.
	b, _ := json.Marshal(p)
	if e := d.writeBox(c.ID, "bootstrap.json", b); e != nil {
		return errors.New("channel_not_integrated")
	}
	d.Rec.Pair = p
	return save(filepath.Join(d.Root, "registration.json"), d.Rec)
}
func (d *Docker) challenge() error {
	p := d.Rec.Pair
	b, e := d.readBox(p.Container, "ready.json", 4096)
	if e != nil {
		return errors.New("channel_not_integrated")
	}
	var got Pair
	if verify(p.Key, "ready", b, &got) != nil || got != p {
		return errors.New("channel_not_integrated")
	}
	return nil
}
func digest(b []byte) string { h := sha256.Sum256(b); return "sha256:" + hex.EncodeToString(h[:]) }
func validDigest(s string) bool {
	if len(s) != 71 || !strings.HasPrefix(s, "sha256:") {
		return false
	}
	_, e := hex.DecodeString(s[7:])
	return e == nil
}
func (d *Docker) resolve(tag string) (Release, error) {
	r := Release{Tag: tag, Platform: d.Rec.Platform}
	if !tagPattern.MatchString(tag) {
		return r, errors.New("invalid_release")
	}
	// Buildx talks to the registry with the original Docker client's credentials.
	b, e := d.run(false, nil, 2<<20, "buildx", "imagetools", "inspect", repository+":"+tag, "--format", "{{json .Manifest}}")
	if e != nil {
		return r, e
	}
	var descriptor struct{ Digest string }
	if json.Unmarshal(b, &descriptor) != nil || !validDigest(descriptor.Digest) {
		return r, errors.New("release_descriptor")
	}
	r.IndexDigest = descriptor.Digest
	b, e = d.run(false, nil, 2<<20, "buildx", "imagetools", "inspect", repository+"@"+r.IndexDigest, "--raw")
	if e != nil {
		return r, e
	}
	var index struct {
		Manifests []struct {
			Digest   string
			Platform struct{ OS, Architecture, Variant string }
		}
		Config struct{ Digest string }
	}
	if json.Unmarshal(b, &index) != nil {
		return r, errors.New("release_manifest")
	}
	r.Digest = r.IndexDigest
	if len(index.Manifests) > 0 {
		matches := 0
		for _, m := range index.Manifests {
			if m.Platform.OS+"/"+m.Platform.Architecture == r.Platform && (m.Platform.Variant == "" || m.Platform.Variant == "v8") {
				r.Digest = m.Digest
				matches++
			}
		}
		if matches != 1 || !validDigest(r.Digest) {
			return r, errors.New("release_platform")
		}
		b, e = d.run(false, nil, 2<<20, "buildx", "imagetools", "inspect", repository+"@"+r.Digest, "--raw")
		if e != nil {
			return r, e
		}
		if json.Unmarshal(b, &index) != nil {
			return r, errors.New("release_manifest")
		}
	}
	r.ImageID = index.Config.Digest
	if !validDigest(r.ImageID) {
		return r, errors.New("release_config_digest")
	}
	return r, nil
}
func (d *Docker) image(r Release) error {
	b, e := d.run(false, nil, 2<<20, "image", "inspect", repository+"@"+r.Digest)
	if e != nil {
		return e
	}
	var got []struct {
		ID               string `json:"Id"`
		RepoDigests      []string
		OS, Architecture string
		Config           struct{ Labels map[string]string }
	}
	if json.Unmarshal(b, &got) != nil || len(got) != 1 {
		return errors.New("image_inspect")
	}
	g := got[0]
	member := false
	for _, s := range g.RepoDigests {
		if s == repository+"@"+r.Digest {
			member = true
		}
	}
	if !member || g.ID != r.ImageID || g.OS+"/"+g.Architecture != r.Platform || g.Config.Labels["version"] != r.Tag {
		return errors.New("image_release_mismatch")
	}
	return nil
}
func (d *Docker) pull(r Release, deadline int64) error {
	_, e := d.R.Run(d.args("pull", "--platform", r.Platform, repository+"@"+r.Digest), nil, deadline, 1<<20, true)
	return e
}
func (d *Docker) dependencies() error {
	names := make([]string, 0, len(d.Rec.Containers))
	for n := range d.Rec.Containers {
		if n != d.Rec.Invocation.Service {
			names = append(names, n)
		}
	}
	sort.Strings(names)
	for _, n := range names {
		before := d.Rec.Containers[n]
		now, e := d.container(n)
		if e != nil {
			return e
		}
		if before.ID != now.ID || before.Image != now.Image || before.Config.Labels["com.docker.compose.config-hash"] != now.Config.Labels["com.docker.compose.config-hash"] || !sameRuntime(before, now) {
			return errors.New("dependency_changed")
		}
		if !now.State.Running {
			if _, e = d.comp(false, true, "start", n); e != nil {
				return e
			}
			return errors.New("dependencies_waiting")
		}
		if now.State.Health.Status != "healthy" {
			return errors.New("dependencies_waiting")
		}
	}
	return nil
}
func (d *Docker) selectRelease(r Release, generation int) error {
	v := map[string]any{"services": map[string]any{d.Rec.Invocation.Service: map[string]any{"image": repository + "@" + r.Digest, "environment": map[string]string{"VECTORAIZ_VERSION": r.Tag,"VECTORAIZ_OWNER_MAILBOX":d.mailboxPath()}}}}
	if e := save(filepath.Join(d.Root, "selection.compose.json"), v); e != nil {
		return e
	}
	return save(filepath.Join(d.Root, "selection.json"), map[string]any{"release": r, "generation": generation})
}
func (d *Docker) observed(r Release, prior string) (Identity, error) {
	c, e := d.container(d.Rec.Invocation.Service)
	if e != nil {
		return c, e
	}
	base := d.Rec.Containers[d.Rec.Invocation.Service]
	if !sameRuntime(base, c) {
		return c, errors.New("app_runtime_changed")
	}
	if c.Image != r.ImageID || base.Image != r.ImageID && c.ID == prior {
		return c, errors.New("wrong_image")
	}
	if !c.State.Running || c.State.Health.Status != "healthy" {
		return c, errors.New("startup_waiting")
	}
	if c.Config.Labels["version"] != r.Tag {
		return c, errors.New("wrong_build_version")
	}
	found := false
	for _, v := range c.Config.Env {
		if v == "VECTORAIZ_VERSION="+r.Tag {
			found = true
		}
	}
	if !found {
		return c, errors.New("wrong_selected_version")
	}
	if e = d.appConfig(c, true); e != nil {
		return c, e
	}
	// Execute a fixed curl inside the pinned container, not an unrelated browser port.
	b, e := d.run(false, nil, 32768, "exec", c.ID, "curl", "--fail", "--silent", "--max-time", "5", "http://127.0.0.1/api/health")
	if e != nil {
		return c, errors.New("health_unreachable")
	}
	var h struct {
		Status  string
		Version string
	}
	if json.Unmarshal(b, &h) != nil || h.Status != "healthy" && h.Status != "ok" || strings.TrimPrefix(h.Version, "v") != strings.TrimPrefix(r.Tag, "v") {
		return c, errors.New("reported_health_version_mismatch")
	}
	return c, nil
}
func (d *Docker) appConfig(c Identity, selected bool) error {
	b, e := d.comp(selected, false, "config", "--hash", d.Rec.Invocation.Service)
	if e != nil {
		return e
	}
	v := strings.Fields(string(b))
	if len(v) != 2 || v[0] != d.Rec.Invocation.Service || v[1] != c.Config.Labels["com.docker.compose.config-hash"] {
		return errors.New("actual_app_configuration_mismatch")
	}
	return nil
}
func absolute(cwd, p string) string {
	if !filepath.IsAbs(p) {
		p = filepath.Join(cwd, p)
	}
	p, _ = filepath.Abs(p)
	return filepath.Clean(p)
}
func register(root string, i Invocation, r Runner) (*Record, error) {
	if i.Channel != "direct" && i.Channel != "marketplace" {
		return nil, errors.New("channel_not_integrated")
	}
	if len(i.Files) == 0 {
		return nil, errors.New("original_compose_files_required")
	}
	if i.Service == "" {
		i.Service = "vectoraiz"
	}
	if i.Docker == "" {
		i.Docker = "docker"
	}
	if i.Cwd == "" {
		i.Cwd, _ = os.Getwd()
	}
	i.Cwd = absolute(i.Cwd, ".")
	if i.ProjectDirectory == "" {
		i.ProjectDirectory = filepath.Dir(absolute(i.Cwd, i.Files[0]))
	}
	i.ProjectDirectory = absolute(i.Cwd, i.ProjectDirectory)
	for n, p := range i.Files {
		i.Files[n] = absolute(i.Cwd, p)
	}
	for n, p := range i.EnvFiles {
		i.EnvFiles[n] = absolute(i.Cwd, p)
	}
	rec := &Record{Invocation: i, Installation: token(), Sources: map[string]string{}, Containers: map[string]Identity{}, Hashes: map[string]string{}}
	d := Docker{R: r, Rec: rec, Root: root}
	b, e := d.comp(false, false, "config", "--format", "json")
	if e != nil {
		return nil, e
	}
	var raw struct {
		Name     string
		Services map[string]struct {
			Image       string
			Environment map[string]string
			EnvFile     []any
		}
		Secrets, Configs map[string]struct{ File string }
	}
	if json.Unmarshal(b, &raw) != nil {
		return nil, errors.New("original_config_unresolved")
	}
	if i.Project == "" {
		rec.Invocation.Project = raw.Name
	}
	if rec.Invocation.Project == "" {
		return nil, errors.New("original_project_unresolved")
	}
	rec.Original = append([]byte{}, b...)
	rec.Effective, e = normalizeConfig(b, i.Service)
	if e != nil {
		return nil, e
	}
	// Original app image must belong to this channel, not AIM-Data or a new project.
	if !strings.HasPrefix(raw.Services[i.Service].Image, repository+":") && !strings.HasPrefix(raw.Services[i.Service].Image, repository+"@") {
		return nil, errors.New("channel_image_mismatch")
	}
	sources := append(append([]string{}, i.Files...), i.EnvFiles...)
	if i.DefaultEnv {
		originalEnv:=map[string]string{};for _,s:=range i.Environment{k,v,ok:=strings.Cut(s,"=");if ok{originalEnv[k]=v}}
		if paths:=originalEnv["COMPOSE_ENV_FILES"];paths!=""{for _,p:=range strings.Split(paths,","){sources=append(sources,absolute(i.Cwd,p))}}else if originalEnv["COMPOSE_DISABLE_ENV_FILE"]!="1"&&originalEnv["COMPOSE_DISABLE_ENV_FILE"]!="true"{if _,e=os.Stat(filepath.Join(i.Cwd,".env"));e==nil{sources=append(sources,filepath.Join(i.Cwd,".env"))}}
		if i.Cwd != i.ProjectDirectory {
			if _, e = os.Stat(filepath.Join(i.ProjectDirectory, ".env")); e == nil {
				sources = append(sources, filepath.Join(i.ProjectDirectory, ".env"))
			}
		}
	}
	// config without env resolution retains service env_file credential origins.
	eb, e := d.comp(false, false, "config", "--no-env-resolution", "--format", "json")
	if e != nil {
		return nil, e
	}
	var origins struct {
		Services         map[string]struct{ EnvFile []any }
		Secrets, Configs map[string]struct{ File string }
	}
	if json.Unmarshal(eb, &origins) != nil {
		return nil, errors.New("source_origins_unresolved")
	}
	for _, s := range origins.Services {
		for _, f := range s.EnvFile {
			p, _ := f.(string)
			if obj, ok := f.(map[string]any); ok {
				p, _ = obj["path"].(string)
			}
			if p != "" {
				sources = append(sources, absolute(i.ProjectDirectory, p))
			}
		}
	}
	for _, items := range []map[string]struct{ File string }{origins.Secrets, origins.Configs} {
		for _, v := range items {
			if v.File != "" {
				sources = append(sources, absolute(i.ProjectDirectory, v.File))
			}
		}
	}
	for _, p := range sources {
		stamp, e := ownership(p)
		if e != nil {
			return nil, fmt.Errorf("original_source_unavailable: %s", filepath.Base(p))
		}
		rec.Sources[p] = stamp
	}
	rec.Endpoint, e = d.endpoint()
	if e != nil {
		return nil, e
	}
	if e = d.captureCredentials(); e != nil {
		return nil, e
	}
	b, e = d.run(false, nil, 32768, "info", "--format", "{{.OSType}}/{{.Architecture}}")
	if e != nil {
		return nil, e
	}
	rec.Platform = strings.TrimSpace(string(b))
	rec.Platform = strings.ReplaceAll(rec.Platform, "x86_64", "amd64")
	rec.Platform = strings.ReplaceAll(rec.Platform, "aarch64", "arm64")
	if rec.Platform != "linux/amd64" && rec.Platform != "linux/arm64" {
		return nil, errors.New("platform_not_integrated")
	}
	// Compose's own config hash proves that original invocation resolves to the
	// existing containers, instead of reconstructing credentials from inspect.
	b, e = d.comp(false, false, "config", "--hash", "*")
	if e != nil {
		return nil, e
	}
	for _, line := range strings.Split(string(b), "\n") {
		v := strings.Fields(line)
		if len(v) == 2 {
			rec.Hashes[v[0]] = v[1]
		}
	}
	for name := range raw.Services {
		c, e := d.container(name)
		if e != nil {
			return nil, e
		}
		if rec.Hashes[name] == "" || rec.Hashes[name] != c.Config.Labels["com.docker.compose.config-hash"] {
			return nil, errors.New("original_invocation_mismatch")
		}
		rec.Containers[name] = c
	}
	if rec.Mailbox,e = chooseMailbox(rec.Containers[i.Service]); e != nil {
		return nil, e
	}
	// Check only activation flags actually needed, not an arbitrary version gate.
	b, e = d.comp(false, false, "up", "--help")
	if e != nil {
		return nil, e
	}
	for _, flag := range []string{"--no-deps", "--no-build", "--pull"} {
		if !strings.Contains(string(b), flag) {
			return nil, errors.New("compose_activation_unsupported")
		}
	}
	return rec, nil
}
