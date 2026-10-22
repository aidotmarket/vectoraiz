# Fixed SDK for the Windows llama.cpp Vulkan build. No GPU is required by tests.
$ErrorActionPreference = 'Stop'
$version = '1.4.321.1'
$root = "C:\VulkanSDK\$version"
$installer = Join-Path $env:RUNNER_TEMP 'vulkan-sdk.exe'
for ($attempt = 0; $attempt -lt 4; $attempt++) {
    try {
        Invoke-WebRequest -Uri "https://sdk.lunarg.com/sdk/download/$version/windows/vulkansdk-windows-X64-$version.exe" -OutFile $installer
        break
    } catch {
        if ($attempt -eq 3) { throw "Vulkan SDK download failed after 4 attempts: $_" }
        if (Test-Path $installer) { Remove-Item $installer -Force }
        Write-Host "Vulkan SDK download failed; retry $($attempt + 1)/3"
        Start-Sleep -Seconds ([math]::Pow(2, $attempt + 1))
    }
}
$expected = 'baaa4f7ca11ed3d82aa1c102b21208915485bbaa473068c763daa425cca468bd'
if ((Get-FileHash $installer -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expected) { throw 'Vulkan SDK SHA256 mismatch' }
$process = Start-Process -FilePath $installer -ArgumentList @('--root', $root, '--accept-licenses', '--default-answer', '--confirm-command', 'install') -Wait -PassThru
if ($process.ExitCode -ne 0) { throw "Vulkan SDK installer failed: $($process.ExitCode)" }
if (-not (Test-Path "$root\Bin\glslc.exe")) { throw 'Vulkan shader compiler missing' }
"VULKAN_SDK=$root" | Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append
"$root\Bin" | Out-File -FilePath $env:GITHUB_PATH -Encoding utf8 -Append
'NATIVE_FEATURES=--features vz-cli/vulkan' | Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append
'CMAKE_GENERATOR=Ninja' | Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append
