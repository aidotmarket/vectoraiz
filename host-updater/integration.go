package main

import (
	"encoding/xml"
	"errors"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"time"
)

func quoted(s string) string { return "'" + strings.ReplaceAll(s, "'", "'\"'\"'") + "'" }
func xmlText(s string) string {
	var b strings.Builder
	xml.EscapeText(&b, []byte(s))
	return b.String()
}
func unitText(s string) string {
	return `"` + strings.NewReplacer(`\`, `\\`, `"`, `\"`, "%", "%%").Replace(s) + `"`
}
func native(args ...string) error {
	r := Command{Args: args, Env: os.Environ(), Deadline: time.Now().Unix() + 30, Max: 65536}
	return execute(&r)
}
func integrate(root, id string) error {
	exe, e := os.Executable()
	if e != nil {
		return e
	}
	exe = absolute(".", exe)
	home, e := os.UserHomeDir()
	if e != nil {
		return e
	}
	// Keep the exact staged binary. No compiler, Python, RTK or shell interpreter
	// is required by the installed owner's update protocol.
	if e = save(filepath.Join(root, "executable.json"), exe); e != nil {
		return e
	}
	if e = installLaunchers(root, exe, home); e != nil {
		return e
	}
	label := "com.vectoraiz.owner." + id[:16]
	switch runtime.GOOS {
	case "darwin":
		dir := filepath.Join(home, "Library", "LaunchAgents")
		if e = os.MkdirAll(dir, 0700); e != nil {
			return e
		}
		body := `<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>Label</key><string>` + label + `</string><key>ProgramArguments</key><array><string>` + xmlText(exe) + `</string><string>serve</string><string>--root</string><string>` + xmlText(root) + `</string></array><key>RunAtLoad</key><true/><key>KeepAlive</key><true/><key>AbandonProcessGroup</key><true/><key>ThrottleInterval</key><integer>5</integer></dict></plist>`
		return atomicFile(filepath.Join(dir, label+".plist"), []byte(body))
	case "linux":
		dir := filepath.Join(home, ".config", "systemd", "user")
		if e = os.MkdirAll(dir, 0700); e != nil {
			return e
		}
		body := "[Unit]\nDescription=vectorAIz update owner\n[Service]\nType=simple\nExecStart=" + unitText(exe) + " serve --root " + unitText(root) + "\nRestart=always\nRestartSec=5\nKillMode=process\n[Install]\nWantedBy=default.target\n"
		return atomicFile(filepath.Join(dir, label+".service"), []byte(body))
	case "windows":
		// A per-user logon task, no elevation, existing Docker credentials only.
		user := os.Getenv("USERDOMAIN") + `\` + os.Getenv("USERNAME")
		body := `<?xml version="1.0" encoding="UTF-16"?><Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task"><Triggers><LogonTrigger><UserId>` + xmlText(user) + `</UserId></LogonTrigger></Triggers><Principals><Principal id="Author"><UserId>` + xmlText(user) + `</UserId><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals><Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><ExecutionTimeLimit>PT0S</ExecutionTimeLimit><RestartOnFailure><Interval>PT1M</Interval><Count>999</Count></RestartOnFailure></Settings><Actions Context="Author"><Exec><Command>` + xmlText(exe) + `</Command><Arguments>serve --root &quot;` + xmlText(root) + `&quot;</Arguments></Exec></Actions></Task>`
		// schtasks accepts UTF-8 XML with an accurate declaration.
		body = strings.Replace(body, "UTF-16", "UTF-8", 1)
		return atomicFile(filepath.Join(root, "task.xml"), []byte(body))
	}
	return errors.New("platform_not_integrated")
}
func launcherWrite(p string, b []byte, mode os.FileMode) error {
	if old, e := os.ReadFile(p); e == nil {
		backup := p + ".before-owner"
		if _, e = os.Stat(backup); os.IsNotExist(e) {
			if e = atomicFile(backup, old); e != nil {
				return e
			}
		}
	}
	if e := os.MkdirAll(filepath.Dir(p), 0700); e != nil {
		return e
	}
	if e := atomicFile(p, b); e != nil {
		return e
	}
	return os.Chmod(p, mode)
}
func installLaunchers(root, exe, home string) error {
	install := filepath.Dir(root)
	belongs := func(b []byte) bool {
		return strings.Contains(string(b), install) || install == filepath.Join(home, "vectoraiz") && (strings.Contains(string(b), `$HOME/vectoraiz`) || strings.Contains(string(b), `~/vectoraiz`))
	}
	statusAction := "  cat " + quoted(filepath.Join(root, "status.json")) + "\n"
	if runtime.GOOS == "darwin" {
		// LaunchServices does not display stdout. The OS-authorized status file
		// remains usable even when the owner/daemon/app cannot start.
		statusAction += "  /usr/bin/open -a TextEdit " + quoted(filepath.Join(root, "status.json")) + "\n"
	}
	shell := []byte("#!/bin/sh\n" + quoted(exe) + " launch --root " + quoted(root) + " --open\nresult=$?\nif [ \"$result\" -ne 0 ]; then\n" + statusAction + "fi\nexit \"$result\"\n")
	if runtime.GOOS != "windows" {
		if e := launcherWrite(filepath.Join(install, "launch-owner.sh"), shell, 0700); e != nil {
			return e
		}
	}
	// Replace only the shipped launchers, retaining their original bytes privately.
	if runtime.GOOS == "darwin" {
		p := filepath.Join(home, "Applications", "vectorAIz.app", "Contents", "MacOS", "vectorAIz")
		if b, e := os.ReadFile(p); e == nil && belongs(b) {
			if !strings.Contains(string(b), "vectoraiz") {
				return errors.New("unknown_app_launcher")
			}
			if e = launcherWrite(p, shell, 0700); e != nil {
				return e
			}
		}
	}
	if runtime.GOOS == "linux" {
		body := "[Desktop Entry]\nType=Application\nName=vectorAIz\nTerminal=true\nExec=" + unitText(filepath.Join(install, "launch-owner.sh")) + "\n"
		for _, p := range []string{filepath.Join(home, ".local", "share", "applications", "vectoraiz.desktop"), filepath.Join(home, "Desktop", "vectoraiz.desktop")} {
			if b, e := os.ReadFile(p); e == nil && belongs(b) {
				if !strings.Contains(string(b), "vectorAIz") {
					return errors.New("unknown_desktop_launcher")
				}
				if e = launcherWrite(p, []byte(body), 0700); e != nil {
					return e
				}
			}
		}
		p := filepath.Join(home, ".config", "systemd", "user", "vectoraiz.service")
		if b, e := os.ReadFile(p); e == nil && belongs(b) {
			if !strings.Contains(string(b), "vectorAIz") || !strings.Contains(string(b), "Type=oneshot") {
				return errors.New("unknown_autostart_launcher")
			}
			body = "[Unit]\nDescription=vectorAIz launch delegate\n[Service]\nType=oneshot\nExecStart=" + unitText(exe) + " launch --root " + unitText(root) + "\n[Install]\nWantedBy=default.target\n"
			if e = launcherWrite(p, []byte(body), 0600); e != nil {
				return e
			}
		}
	}
	if runtime.GOOS == "windows" {
		psquote := func(s string) string { return "'" + strings.ReplaceAll(s, "'", "''") + "'" }
		body := "& " + psquote(exe) + " launch --root " + psquote(root) + " --open\nif ($LASTEXITCODE -ne 0) { Get-Content -LiteralPath " + psquote(filepath.Join(root, "status.json")) + "; Read-Host 'Read host status above; press Enter to close' }\n"
		path := filepath.Join(install, "launch-vectoraiz.ps1")
		if old, e := os.ReadFile(path); e == nil && !strings.Contains(string(old), "vectorAIz") && !strings.Contains(string(old), " launch --root ") {
			return errors.New("unknown_windows_launcher")
		}
		if e := launcherWrite(path, []byte(body), 0600); e != nil {
			return e
		}
	}
	p := filepath.Join(install, "start.sh")
	if b, e := os.ReadFile(p); e == nil && !strings.Contains(string(b), "registration.json") {
		if !strings.Contains(string(b), "vectorAIz") || !strings.HasPrefix(string(b), "#!/bin/bash\n") {
			return errors.New("unknown_root_launcher")
		}
		prefix := "#!/bin/bash\nexec " + quoted(exe) + " launch --root " + quoted(root) + " --open\n"
		if e = launcherWrite(p, []byte(prefix+strings.TrimPrefix(string(b), "#!/bin/bash\n")), 0700); e != nil {
			return e
		}
	}
	return nil
}
func startSupervisor(root, id string) error {
	home, e := os.UserHomeDir()
	if e != nil {
		return e
	}
	label := "com.vectoraiz.owner." + id[:16]
	switch runtime.GOOS {
	case "darwin":
		job := "gui/" + itoa(os.Getuid()) + "/" + label
		if native("launchctl", "print", job) == nil {
			return native("launchctl", "kickstart", job)
		}
		return native("launchctl", "bootstrap", "gui/"+itoa(os.Getuid()), filepath.Join(home, "Library", "LaunchAgents", label+".plist"))
	case "linux":
		if e = native("systemctl", "--user", "daemon-reload"); e != nil {
			return e
		}
		return native("systemctl", "--user", "enable", "--now", label+".service")
	case "windows":
		if native("schtasks.exe", "/Query", "/TN", label) == nil {
			return native("schtasks.exe", "/Run", "/TN", label)
		}
		if e = native("schtasks.exe", "/Create", "/TN", label, "/XML", filepath.Join(root, "task.xml"), "/F"); e != nil {
			return e
		}
		return native("schtasks.exe", "/Run", "/TN", label)
	}
	return errors.New("platform_not_integrated")
}
func browser(u string) error {
	switch runtime.GOOS {
	case "darwin":
		return native("open", u)
	case "linux":
		return native("xdg-open", u)
	case "windows":
		return native("rundll32.exe", "url.dll,FileProtocolHandler", u)
	}
	return errors.New("browser_unavailable")
}

// Best-effort session notification. Denial/absence never changes the operation
// outcome or removes the already durable file/CLI fallback.
func notifyTerminal(root string) {
	if !allowNativeExecution {
		return
	}
	message := "Update finished. Read the recorded outcome in " + filepath.Join(root, "status.json")
	switch runtime.GOOS {
	case "darwin":
		native("osascript", "-e", "on run argv\ndisplay notification (item 1 of argv) with title \"vectorAIz update\"\nend run", message)
	case "linux":
		native("notify-send", "vectorAIz update", message)
	case "windows":
		r := Command{Args: []string{"powershell.exe", "-NoProfile", "-NonInteractive", "-Command", `Add-Type -AssemblyName System.Windows.Forms; $n=New-Object System.Windows.Forms.NotifyIcon; $n.Icon=[System.Drawing.SystemIcons]::Information; $n.Visible=$true; $n.ShowBalloonTip(4000,'vectorAIz update',$env:VECTORAIZ_OWNER_NOTICE,[System.Windows.Forms.ToolTipIcon]::Info); Start-Sleep -Seconds 4; $n.Dispose()`}, Env: append(os.Environ(), "VECTORAIZ_OWNER_NOTICE="+message), Deadline: time.Now().Unix() + 10, Max: 4096}
		execute(&r)
	}
}
