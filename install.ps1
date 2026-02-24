# =============================================================================
# vectorAIz Installer for Windows
# =============================================================================
# Usage: irm https://raw.githubusercontent.com/maxrobbins/vectoraiz/main/install.ps1 | iex
#
# Requires: Docker Desktop (with WSL2 backend)
# =============================================================================

$ErrorActionPreference = "Stop"

function Write-Info  { param($msg) Write-Host "  ▸ $msg" -ForegroundColor Cyan }
function Write-Ok    { param($msg) Write-Host "  ✓ $msg" -ForegroundColor Green }
function Write-Err   { param($msg) Write-Host "`n  ERROR: $msg`n" -ForegroundColor Red; exit 1 }

$repo   = "maxrobbins/vectoraiz"
$branch = "main"
$installDir = "$HOME\vectoraiz"

Write-Host ""
Write-Host "  ⚡ vectorAIz Installer" -ForegroundColor Cyan
Write-Host ""

# ─── Check Docker ────────────────────────────────────────────────
Write-Info "Checking prerequisites..."

$dockerCmd = Get-Command docker -ErrorAction SilentlyContinue
if (-not $dockerCmd) {
    Write-Host ""
    Write-Host "  Docker is not installed." -ForegroundColor Red
    Write-Host ""
    Write-Host "  vectorAIz runs in Docker containers. Please install:"
    Write-Host ""
    Write-Host "    Docker Desktop:  https://docs.docker.com/desktop/install/windows/" -ForegroundColor White
    Write-Host "    (Use the WSL2 backend — it's the default)" -ForegroundColor DarkGray
    Write-Host ""
    Write-Host "  After installing Docker Desktop, restart your terminal and run this again."
    Write-Host ""
    exit 1
}

$dockerInfo = docker info 2>&1
if ($LASTEXITCODE -ne 0) {
    Write-Err "Docker is installed but not running. Please start Docker Desktop first."
}

Write-Ok "Docker is running"

# ─── Download ────────────────────────────────────────────────────
$zipUrl  = "https://github.com/$repo/archive/refs/heads/$branch.zip"
$tmpDir  = Join-Path $env:TEMP "vectoraiz-install-$(Get-Random)"
$zipFile = Join-Path $tmpDir "vectoraiz.zip"

New-Item -ItemType Directory -Path $tmpDir -Force | Out-Null

if (Test-Path "$installDir\backend") {
    Write-Info "Existing installation found at $installDir — updating..."
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

# Copy to install dir
if (-not (Test-Path $installDir)) {
    New-Item -ItemType Directory -Path $installDir -Force | Out-Null
}

Copy-Item -Path "$($extractedDir.FullName)\*" -Destination $installDir -Recurse -Force

if ($envBackup) {
    Set-Content -Path "$installDir\backend\.env" -Value $envBackup
    Write-Ok "Preserved existing configuration"
}

# Cleanup temp
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

    # Check if port 80 is free
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

# Start containers
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

if ($waited -ge $maxWait) {
    Write-Host "  ⚠ Timed out. Try opening http://localhost:$port in a minute." -ForegroundColor Yellow
} else {
    Write-Ok "All services healthy"
}

# Open browser
$url = if ($port -eq "80") { "http://localhost" } else { "http://localhost:$port" }

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
