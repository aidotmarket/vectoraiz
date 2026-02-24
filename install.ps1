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

# Spinner — runs a script block with animated spinner
function Invoke-WithSpinner {
    param(
        [string]$Message,
        [scriptblock]$Action
    )
    $frames = @('⠋','⠙','⠹','⠸','⠼','⠴','⠦','⠧','⠇','⠏')
    $job = Start-Job -ScriptBlock $Action
    $i = 0

    while ($job.State -eq 'Running') {
        $frame = $frames[$i % $frames.Count]
        Write-Host "`r  $frame $Message" -NoNewline -ForegroundColor Cyan
        Start-Sleep -Milliseconds 100
        $i++
    }

    $result = Receive-Job $job
    Remove-Job $job -Force
    Write-Host "`r                                                                        `r" -NoNewline
    return $result
}

# Waiting spinner — spins while checking a condition
function Wait-WithSpinner {
    param(
        [string]$Message,
        [int]$MaxSeconds = 120,
        [scriptblock]$Check
    )
    $frames = @('⠋','⠙','⠹','⠸','⠼','⠴','⠦','⠧','⠇','⠏')
    $waited = 0
    $i = 0

    while ($waited -lt $MaxSeconds) {
        try {
            $ok = & $Check 2>$null
            if ($ok -ne $false -and $LASTEXITCODE -eq 0) {
                Write-Host "`r                                                                        `r" -NoNewline
                return $true
            }
        } catch { }

        $frame = $frames[$i % $frames.Count]
        Write-Host "`r  $frame $Message ($($waited)s)" -NoNewline -ForegroundColor Cyan
        Start-Sleep -Milliseconds 500
        $i++
        if ($i % 2 -eq 0) { $waited++ }
    }

    Write-Host "`r                                                                        `r" -NoNewline
    return $false
}

$repo   = "maxrobbins/vectoraiz"
$branch = "main"
$installDir = "$HOME\vectoraiz"

Write-Host ""
Write-Host "  ⚡ vectorAIz Installer" -ForegroundColor Cyan
Write-Host ""

# ─── Install Docker if missing ───────────────────────────────────
function Install-DockerDesktop {
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
            Write-Warn "winget had an issue. Trying direct download..."
        }
    }

    # Fallback: direct download with spinner
    $installerUrl = "https://desktop.docker.com/win/main/amd64/Docker%20Desktop%20Installer.exe"
    $installerPath = Join-Path $env:TEMP "DockerDesktopInstaller.exe"

    Invoke-WithSpinner -Message "Downloading Docker Desktop..." -Action {
        [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
        Invoke-WebRequest -Uri "https://desktop.docker.com/win/main/amd64/Docker%20Desktop%20Installer.exe" -OutFile (Join-Path $env:TEMP "DockerDesktopInstaller.exe") -UseBasicParsing
    }

    if (-not (Test-Path $installerPath)) {
        Write-Err "Failed to download Docker Desktop. Install manually: https://docker.com/get-started"
    }

    Write-Ok "Downloaded Docker Desktop"
    Write-Info "Running installer (follow any prompts)..."
    Write-Host ""

    Start-Process -FilePath $installerPath -ArgumentList "install", "--quiet", "--accept-license" -Wait

    Remove-Item $installerPath -Force -ErrorAction SilentlyContinue
    Write-Ok "Docker Desktop installed"
    return $true
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
        $env:Path = [System.Environment]::GetEnvironmentVariable("Path", "Machine") + ";" + [System.Environment]::GetEnvironmentVariable("Path", "User")

        $dockerDesktop = "C:\Program Files\Docker\Docker\Docker Desktop.exe"
        if (Test-Path $dockerDesktop) {
            Start-Process $dockerDesktop
        }

        Write-Host "  (First launch takes 30-60 seconds)" -ForegroundColor DarkGray

        $ready = Wait-WithSpinner -Message "Starting Docker Desktop..." -MaxSeconds 120 -Check { docker info }

        if (-not $ready) {
            Write-Host ""
            Write-Warn "Docker is still starting up."
            Write-Warn "Wait for it to finish, then re-run:"
            Write-Host ""
            Write-Host "    irm https://raw.githubusercontent.com/maxrobbins/vectoraiz/main/install.ps1 | iex" -ForegroundColor White
            Write-Host ""
            exit 0
        }
    }
} else {
    $info = docker info 2>&1
    if ($LASTEXITCODE -ne 0) {
        $dockerDesktop = "C:\Program Files\Docker\Docker\Docker Desktop.exe"
        if (Test-Path $dockerDesktop) {
            Start-Process $dockerDesktop
        }

        $ready = Wait-WithSpinner -Message "Starting Docker Desktop..." -MaxSeconds 90 -Check { docker info }
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

Invoke-WithSpinner -Message "Downloading vectorAIz..." -Action {
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    $tmpDir = Join-Path $env:TEMP (Get-ChildItem $env:TEMP -Directory -Filter "vectoraiz-install-*" | Select-Object -Last 1).Name
    $zipFile = Join-Path $tmpDir "vectoraiz.zip"
    Invoke-WebRequest -Uri "https://github.com/maxrobbins/vectoraiz/archive/refs/heads/main.zip" -OutFile $zipFile -UseBasicParsing
}

if (-not (Test-Path $zipFile)) {
    Write-Err "Failed to download. Check your internet connection."
}

Write-Ok "Downloaded vectorAIz"

# ─── Extract ─────────────────────────────────────────────────────
Write-Info "Extracting..."

Expand-Archive -Path $zipFile -DestinationPath $tmpDir -Force

$extractedDir = Get-ChildItem -Path $tmpDir -Directory -Filter "vectoraiz*" | Select-Object -First 1
if (-not $extractedDir) {
    Write-Err "Failed to extract archive."
}

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

$url = if ($port -eq "80") { "http://localhost" } else { "http://localhost:$port" }

$ready = Wait-WithSpinner -Message "Waiting for vectorAIz to be ready..." -MaxSeconds 180 -Check {
    try { (Invoke-WebRequest -Uri "http://localhost:$using:port/api/health" -UseBasicParsing -TimeoutSec 3).StatusCode -eq 200 } catch { $false }
}

if (-not $ready) {
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
