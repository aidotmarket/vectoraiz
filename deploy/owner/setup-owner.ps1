param([string]$InstallDir, [string]$ComposeFile, [string]$Release, [string]$AppUrl, [string]$Channel = 'direct')
$ErrorActionPreference = 'Stop'
if ($Channel -eq 'aim-data') { throw 'channel_not_integrated: AIM-Data requires its own release evidence.' }
$state = Join-Path $InstallDir '.vectoraiz-owner'
$owner = Join-Path $state 'owner.exe'
if (Test-Path (Join-Path $state 'registration.json')) {
    & $owner launch --root $state --open
    if ($LASTEXITCODE -ne 0) { throw 'Owner launch needs recovery; read status.json.' }
    return
}
if ($Release -notmatch '^v?\d+\.\d+\.\d+(-rc\.\d+)?$') { throw 'An exact verified owner release is required.' }
if ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne 'X64') { throw 'This Windows installer ships the amd64 owner only.' }
New-Item -ItemType Directory -Path $state -Force | Out-Null
$sid = [System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value
# Restrict inherited ACLs before any private state/keys are written.
& icacls.exe $state /inheritance:r /grant:r "*${sid}:(OI)(CI)F" '*S-1-5-18:(OI)(CI)F' | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'Cannot protect owner state ACL.' }
$asset = 'vectoraiz-owner-windows-amd64.exe'
$base = "https://github.com/aidotmarket/vectoraiz/releases/download/$Release"
$stage = Join-Path $state ('.package-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $stage | Out-Null
try {
    if ($env:VECTORAIZ_OWNER_PACKAGE_DIR) {
        Copy-Item (Join-Path $env:VECTORAIZ_OWNER_PACKAGE_DIR $asset) (Join-Path $stage $asset)
        Copy-Item (Join-Path $env:VECTORAIZ_OWNER_PACKAGE_DIR 'owner-sha256.txt') (Join-Path $stage 'checksums')
    } else {
        Invoke-WebRequest -Uri "$base/$asset" -OutFile (Join-Path $stage $asset) -UseBasicParsing -TimeoutSec 120
        Invoke-WebRequest -Uri "$base/owner-sha256.txt" -OutFile (Join-Path $stage 'checksums') -UseBasicParsing -TimeoutSec 30
    }
    $lines = @(Get-Content (Join-Path $stage 'checksums') | Where-Object { $_ -match "^[a-f0-9]{64}  $([regex]::Escape($asset))$" })
    if ($lines.Count -ne 1) { throw 'Missing or ambiguous owner checksum.' }
    $expected = $lines[0].Substring(0, 64)
    if ((Get-FileHash (Join-Path $stage $asset) -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expected) { throw 'Owner checksum mismatch.' }
    $packageVersion = & (Join-Path $stage $asset) version
    if ($LASTEXITCODE -ne 0 -or $packageVersion -ne $Release) { throw 'Owner belongs to another release.' }
    Move-Item (Join-Path $stage $asset) $owner -Force
    Push-Location $InstallDir
    try {
        & $owner setup --root $state --cwd $InstallDir --file (Join-Path $InstallDir $ComposeFile) --channel $Channel --target $Release --url $AppUrl
        if ($LASTEXITCODE -ne 0) { throw 'Adoption needs recovery; preserve original context and read status.json.' }
        & $owner wait --root $state
        if ($LASTEXITCODE -ne 0) { throw 'Adoption has not verified success; read status.json and keep original configuration/data.' }
    } finally { Pop-Location }
} finally { Remove-Item $stage -Recurse -Force }
Write-Host "Adoption accepted. Independent status: $state\status.json"
