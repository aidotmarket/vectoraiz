# =============================================================================
# vectorAIz — Windows Uninstaller
# =============================================================================

$ErrorActionPreference = "Stop"

# --- Configuration ---
$InstallDir = "$env:USERPROFILE\vectoraiz"
$ComposeFile = "docker-compose.customer.yml"

# --- Helpers ---
function Write-Info {
    param([string]$Message)
    Write-Host "  ▸ $Message" -ForegroundColor Blue
}

function Write-Success {
    param([string]$Message)
    Write-Host "  ✓ $Message" -ForegroundColor Green
}

function Write-Warn {
    param([string]$Message)
    Write-Host "  ⚠ $Message" -ForegroundColor Yellow
}

Write-Host ""
Write-Host "  ╔═══════════════════════════════════════════╗" -ForegroundColor Cyan
Write-Host "  ║                                           ║" -ForegroundColor Cyan
Write-Host "  ║     vectorAIz — Windows Uninstaller       ║" -ForegroundColor Cyan
Write-Host "  ║                                           ║" -ForegroundColor Cyan
Write-Host "  ╚═══════════════════════════════════════════╝" -ForegroundColor Cyan
Write-Host ""

# ─── Step 1: Stop containers ─────────────────────────────────────
$composeFilePath = "$InstallDir\$ComposeFile"
$dockerAvailable = $false
try {
    $null = docker info 2>&1
    if ($LASTEXITCODE -eq 0) { $dockerAvailable = $true }
} catch {}

if ((Test-Path $composeFilePath) -and $dockerAvailable) {
    Write-Info "Stopping vectorAIz containers..."
    Set-Location $InstallDir
    docker compose -f $ComposeFile down 2>&1 | Out-Null
    Write-Success "Containers stopped"
} else {
    Write-Info "No running containers found (skipping)"
}

# ─── Step 2: Ask about volumes ───────────────────────────────────
Write-Host ""
Write-Host "  WARNING: Removing Docker volumes will permanently delete all your data" -ForegroundColor Yellow
Write-Host "  (uploaded files, databases, vector indices, user accounts)" -ForegroundColor DarkGray
Write-Host ""
$deleteVolumes = Read-Host "  Delete all data volumes? [y/N]"
Write-Host ""

if ($deleteVolumes -eq "y" -or $deleteVolumes -eq "Y") {
    if ($dockerAvailable -and (Test-Path $composeFilePath)) {
        Write-Info "Removing Docker volumes..."
        Set-Location $InstallDir
        docker compose -f $ComposeFile down -v 2>&1 | Out-Null
        Write-Success "Docker volumes removed"
    }
} else {
    Write-Info "Keeping Docker volumes (your data is preserved)"
}

# ─── Step 3: Remove shortcuts ────────────────────────────────────
$desktopShortcut = [Environment]::GetFolderPath("Desktop") + "\vectorAIz.lnk"
if (Test-Path $desktopShortcut) {
    Remove-Item $desktopShortcut -Force
    Write-Success "Removed desktop shortcut"
}

$startMenuShortcut = "$env:APPDATA\Microsoft\Windows\Start Menu\Programs\vectorAIz.lnk"
if (Test-Path $startMenuShortcut) {
    Remove-Item $startMenuShortcut -Force
    Write-Success "Removed Start Menu shortcut"
}

# ─── Step 4: Remove install directory ─────────────────────────────
if (Test-Path $InstallDir) {
    Write-Info "Removing install directory..."
    Remove-Item $InstallDir -Recurse -Force
    Write-Success "Removed $InstallDir"
} else {
    Write-Info "Install directory not found (skipping)"
}

Write-Host ""
Write-Host "  vectorAIz has been uninstalled." -ForegroundColor Green
Write-Host ""
if ($deleteVolumes -ne "y" -and $deleteVolumes -ne "Y") {
    Write-Host "  Note: Docker volumes were preserved. To fully clean up:" -ForegroundColor DarkGray
    Write-Host "  docker volume rm vectoraiz_vectoraiz-data vectoraiz_postgres-data vectoraiz_qdrant-storage" -ForegroundColor DarkGray
    Write-Host ""
}
