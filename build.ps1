[CmdletBinding()]
param(
    [switch]$Clean
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $repoRoot

# Rust's GNU Windows toolchain needs the w64devkit linker and libraries.
$toolchainRoots = @(
    (Join-Path $env:LOCALAPPDATA 'w64devkit\w64devkit'),
    (Join-Path $env:USERPROFILE '.cache\rustup\w64devkit')
)

foreach ($toolchainRoot in $toolchainRoots) {
    $toolchainBin = Join-Path $toolchainRoot 'bin'
    if (Test-Path (Join-Path $toolchainBin 'x86_64-w64-mingw32-gcc.exe')) {
        $env:PATH = "$toolchainBin;$env:PATH"
        break
    }
}

if ($Clean) {
    & cargo clean
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}

& cargo build --release
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$artifact = Join-Path $repoRoot 'target\release\think.exe'
$distributionDirectory = Join-Path $repoRoot 'dist'
$output = Join-Path $distributionDirectory 'Think.exe'

if (-not (Test-Path $artifact)) {
    throw "Build completed but did not create $artifact"
}

New-Item -ItemType Directory -Force -Path $distributionDirectory | Out-Null
Copy-Item -Force -Path $artifact -Destination $output
Write-Host "Built $output"
