# =============================================================================
# vectorAIz Installer for Windows
# =============================================================================
# Usage: irm https://raw.githubusercontent.com/maxrobbins/vectoraiz/main/install.ps1 | iex
#
# Installs Docker Desktop automatically if missing (via winget).
# =============================================================================

$ErrorActionPreference = "Stop"

function Write-Info  { param($msg) Write-Host "  > $msg" -ForegroundColor Cyan }
function Write-Ok    { param($msg) Write-Host "  ✓ $msg" -ForegroundColor Green }
function Write-Warn  { param($msg) Write-Host "  ! $msg" -ForegroundColor Yellow }
function Write-Err   { param($msg) Write-Host "`n  ERROR: $msg`n" -ForegroundColor Red; exit 1 }

$repo   = "maxrobbins/vectoraiz"
$branch = "main"
$installDir = "$HOME\vectoraiz"

Write-Host ""
Write-Host "  ⚡ vectorAIz Installer" -ForegroundColor Cyan
Write-Host ""

# ─── Install Docker if missing ───────────────────────────────────
function Install-DockerDesktop {
    # Try winget first (ships with Windows 10 1809+ and Windows 11)
    $winget = Get-Command winget -ErrorAction SilentlyContinue
    if ($winget) {
        Write-Info "Installing Docker Desktop via winget..."
        Write-Host "  (This may take a few minutes)" -ForegroundColor DarkGray
        Write-Host ""

        winget install --id Docker.DockerDesktop --accept-source-agreements --accept-package-agreements 2>&1 | ForEach-Object {
            if ($_ -match "Successfully|Found|Installing|Downloaded") {
                Write-Host "  | $_" -ForegroundColor DarkGray
            }
        }

        if ($LASTEXITCODE -eq 0) {
            Write-Ok "Docker Desktop installed"
            return $true
        } else {
            Write-Warn "winget install returned an error. Trying direct download..."
        }
    }

    # Fallback: direct download
    Write-Info "Downloading Docker Desktop installer..."
    $installerUrl = "https://desktop.docker.com/win/main/amd64/Docker%20Desktop%20Installer.exe"
    $installerPath = Join-Path $env:TEMP "DockerDesktopInstaller.exe"

    try {
        [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
        Invoke-WebRequest -Uri $installerUrl -OutFile $installerPath -UseBasicParsing
    } catch {
        Write-Err "Failed to download Docker Desktop. Install manually: https://docker.com/get-started"
    }

    Write-Ok "Downloaded"
    Write-Info "Running Docker Desktop installer..."
    Write-Host "  (Follow the installer prompts)" -ForegroundColor DarkGray
    Write-Host ""

    Start-Process -FilePath $installerPath -ArgumentList "install", "--quiet", "--accept-license" -Wait

    Remove-Item $installerPath -Force -ErrorAction SilentlyContinue
    Write-Ok "Docker Desktop installed"
    return $true
}

function Wait-ForDocker {
    param([int]$MaxWait = 120)

    Write-Info "Waiting for Docker to start..."
    Write-Host "  (This takes 30-60 seconds on first launch)" -ForegroundColor DarkGray

    $waited = 0
    while ($waited -lt $MaxWait) {
        $info = docker info 2>&1
        if ($LASTEXITCODE -eq 0) {
            Write-Host ""
            Write-Ok "Docker is ready"
            return $true
        }
        Start-Sleep -Seconds 3
        $waited += 3
        Write-Host "`r  ⏳ Waiting for Docker... ($($waited)s)" -NoNewline
    }

    Write-Host ""
    return $false
}

$dockerCmd = Get-Command docker -ErrorAction SilentlyContinue
if (-not $dockerCmd) {
    Write-Host ""
    Write-Info "Docker is not installed. vectorAIz needs Docker to run."
    Write-Host ""

    $reply = Read-Host "  Install Docker Desktop now? [Y/n]"
    if (-not $reply) { $reply = "y" }

    if ($reply -match "^[Nn]") {
        Write-Host ""
        Write-Info "Install Docker Desktop manually:"
        Write-Host "    https://docs.docker.com/desktop/install/windows/" -ForegroundColor White
        Write-Host ""
        Write-Info "Then re-run this installer."
        Write-Host ""
        exit 0
    }

    $installed = Install-DockerDesktop

    if ($installed) {
        # Refresh PATH so docker command is found
        $env:Path = [System.Environment]::GetEnvironmentVariable("Path", "Machine") + ";" + [System.Environment]::GetEnvironmentVariable("Path", "User")

        # Try to start Docker Desktop
        $dockerDesktop = "C:\Program Files\Docker\Docker\Docker Desktop.exe"
        if (Test-Path $dockerDesktop) {
            Write-Info "Starting Docker Desktop..."
            Start-Process $dockerDesktop
        }

        $ready = Wait-ForDocker -MaxWait 120

        if (-not $ready) {
            Write-Host ""
            Write-Warn "Docker is still starting up."
            Write-Warn "Please wait for Docker Desktop to finish loading, then re-run:"
            Write-Host ""
            Write-Host "    irm https://raw.githubusercontent.com/maxrobbins/vectoraiz/main/install.ps1 | iex" -ForegroundColor White
            Write-Host ""
            exit 0
        }
    }
} else {
    # Docker command exists — check if running
    $info = docker info 2>&1
    if ($LASTEXITCODE -ne 0) {
        Write-Info "Docker is installed but not running. Starting it..."

        $dockerDesktop = "C:\Program Files\Docker\Docker\Docker Desktop.exe"
        if (Test-Path $dockerDesktop) {
            Start-Process $dockerDesktop
        }

        $ready = Wait-ForDocker -MaxWait 90
        if (-not $ready) {
            Write-Err "Docker didn't start. Please start Docker Desktop manually and re-run."
        }
    }
}

Write-Ok "Docker is ready"

# ─── Download ────────────────────────────────────────────────────
$zipUrl  = "https://github.com/$repo/archive/refs/heads/$branch.zip"
$tmpDir  = Join-Path $env:TEMP "vectoraiz-install-$(Get-Random)"
$zipFile = Join-Path $tmpDir "vectoraiz.zip"

New-Item -ItemType Directory -Path $tmpDir -Force | Out-Null

if (Test-Path "$installDir\backend") {
    Write-Info "Existing installation found — updating..."
    $existing = $true
} else {
    $existing = $false
}

Write-Info "Downloading vectorAIz..."

try {
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    Invoke-WebRequest -Uri $zipUrl -OutFile $zipFile -UseBasicParsing
} catch {
    Write-Err "Failed to download. Check your internet connection."
}

Write-Ok "Downloaded"

# ─── Extract ─────────────────────────────────────────────────────
Write-Info "Extracting..."

Expand-Archive -Path $zipFile -DestinationPath $tmpDir -Force

$extractedDir = Get-ChildItem -Path $tmpDir -Directory -Filter "vectoraiz*" | Select-Object -First 1
if (-not $extractedDir) {
    Write-Err "Failed to extract archive."
}

# Preserve .env if updating
$envBackup = $null
if ($existing -and (Test-Path "$installDir\backend\.env")) {
    $envBackup = Get-Content "$installDir\backend\.env" -Raw
}

if (-not (Test-Path $installDir)) {
    New-Item -ItemType Directory -Path $installDir -Force | Out-Null
}

Copy-Item -Path "$($extractedDir.FullName)\*" -Destination $installDir -Recurse -Force

if ($envBackup) {
    Set-Content -Path "$installDir\backend\.env" -Value $envBackup
    Write-Ok "Preserved existing configuration"
}

Remove-Item -Path $tmpDir -Recurse -Force -ErrorAction SilentlyContinue
Write-Ok "Installed to $installDir"

# ─── Launch ──────────────────────────────────────────────────────
Write-Host ""
Write-Info "Starting vectorAIz..."
Write-Host ""

Set-Location "$installDir\backend"

# Generate .env if first run
if (-not (Test-Path ".env")) {
    $pgPass = -join ((48..57) + (97..122) | Get-Random -Count 32 | ForEach-Object { [char]$_ })
    $port = 80

    $listener = Get-NetTCPConnection -LocalPort 80 -ErrorAction SilentlyContinue
    if ($listener) {
        $port = 8080
        $listener2 = Get-NetTCPConnection -LocalPort 8080 -ErrorAction SilentlyContinue
        if ($listener2) { $port = 3000 }
    }

    @"
# vectorAIz Configuration
# Generated on $(Get-Date -Format "yyyy-MM-dd HH:mm:ss UTC")

POSTGRES_PASSWORD=$pgPass
VECTORAIZ_PORT=$port
VECTORAIZ_MODE=standalone
"@ | Set-Content -Path ".env"

    Write-Ok "Generated .env (port: $port)"
}

$port = (Select-String -Path ".env" -Pattern "^VECTORAIZ_PORT=(\d+)" | ForEach-Object { $_.Matches.Groups[1].Value })
if (-not $port) { $port = "80" }

Write-Info "Building and starting containers (first run may take a few minutes)..."
docker compose -f docker-compose.customer.yml up -d --build

if ($LASTEXITCODE -ne 0) {
    Write-Err "Failed to start containers. Check Docker Desktop is running."
}

# Wait for healthy
Write-Info "Waiting for vectorAIz to be ready..."
$maxWait = 180
$waited = 0

while ($waited -lt $maxWait) {
    try {
        $health = Invoke-WebRequest -Uri "http://localhost:$port/api/health" -UseBasicParsing -TimeoutSec 3 -ErrorAction Stop
        if ($health.StatusCode -eq 200) { break }
    } catch { }
    Start-Sleep -Seconds 3
    $waited += 3
    Write-Host "`r  ⏳ Waiting for services... ($($waited)s)" -NoNewline
}

Write-Host ""

$url = if ($port -eq "80") { "http://localhost" } else { "http://localhost:$port" }

if ($waited -ge $maxWait) {
    Write-Warn "Timed out. Try opening $url in a minute."
} else {
    Write-Ok "All services healthy"

    Write-Host ""
    Write-Host "  ╔═══════════════════════════════════════════╗" -ForegroundColor Green
    Write-Host "  ║                                           ║" -ForegroundColor Green
    Write-Host "  ║          ✅ vectorAIz is Ready!           ║" -ForegroundColor Green
    Write-Host "  ║                                           ║" -ForegroundColor Green
    Write-Host "  ║   Open your browser to:                   ║" -ForegroundColor Green
    Write-Host "  ║   ➜  $url$((' ' * (30 - $url.Length)))║" -ForegroundColor Cyan
    Write-Host "  ║                                           ║" -ForegroundColor Green
    Write-Host "  ╚═══════════════════════════════════════════╝" -ForegroundColor Green
    Write-Host ""

    Start-Process $url
}
