#Requires -Version 7.0
<#
.SYNOPSIS
    Prepare an isolated ONNX Runtime 1.30 CUDA trial for WolfScribe.
.DESCRIPTION
    Downloads the official Microsoft Windows x64 CUDA 13 archive, verifies
    its pinned GitHub release size and SHA-256, and puts its DLLs beside a
    copy of WolfScribe. Does not modify the normal build or system PATH.
#>
[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if (-not $IsWindows -or [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne 'X64') {
    throw 'This CUDA experiment requires Windows x64.'
}

$project = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$release = Join-Path $project 'target\release'
$exe = Join-Path $release 'wolfscribe.exe'
if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) {
    throw "Missing $exe. Build with: cargo build --release --locked --features cuda"
}
if (-not $env:LOCALAPPDATA) { throw 'LOCALAPPDATA is missing.' }

# Pinned data from microsoft/onnxruntime official GitHub v1.30.0 release.
$archiveName = 'onnxruntime-win-x64-gpu_cuda13-1.30.0.zip'
$url = "https://github.com/microsoft/onnxruntime/releases/download/v1.30.0/$archiveName"
$expectedLength = [int64]294386823
$expectedSha256 = '8fa4b08359af682cd605892cb59077049700b640128bb93fd2c7776cf9f55bdc'
$cache = Join-Path $env:LOCALAPPDATA 'WolfScribe\downloads'
$trial = Join-Path $project 'target\blackwell-test'
$archive = Join-Path $cache $archiveName

function Test-Archive {
    if (-not (Test-Path -LiteralPath $archive -PathType Leaf)) { return $false }
    if ((Get-Item -LiteralPath $archive).Length -ne $expectedLength) { return $false }
    Write-Host 'Checking official release SHA-256...'
    return (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant() -eq $expectedSha256
}

New-Item -ItemType Directory -Force -Path $cache, $trial | Out-Null
if (-not (Test-Archive)) {
    if (Test-Path -LiteralPath $archive) {
        Remove-Item -LiteralPath $archive -Force
    }
    Write-Host 'Downloading official Microsoft ONNX Runtime 1.30.0 CUDA 13 (~294 MB)...'
    Write-Host $url
    & curl.exe --fail --location --retry 3 --retry-all-errors --connect-timeout 30 --proto '=https' --proto-redir '=https' --output $archive $url
    if ($LASTEXITCODE -ne 0) { throw "Download failed: curl exit $LASTEXITCODE" }
    if (-not (Test-Archive)) {
        Remove-Item -LiteralPath $archive -Force
        throw 'Archive size or SHA-256 mismatch. Refusing to extract.'
    }
}

$extract = Join-Path $trial '_extract'
if (Test-Path -LiteralPath $extract) { Remove-Item -LiteralPath $extract -Recurse -Force }
New-Item -ItemType Directory -Force -Path $extract | Out-Null
Write-Host 'Extracting verified official archive...'
Expand-Archive -LiteralPath $archive -DestinationPath $extract -Force

$ort = Get-ChildItem -LiteralPath $extract -Recurse -File -Filter 'onnxruntime.dll' |
    Where-Object {
        (Test-Path -LiteralPath (Join-Path $_.DirectoryName 'onnxruntime_providers_cuda.dll')) -and
        (Test-Path -LiteralPath (Join-Path $_.DirectoryName 'onnxruntime_providers_shared.dll'))
    } | Select-Object -First 1

if ($null -eq $ort) { throw 'Matching ONNX Runtime CUDA DLL trio not present in the official archive.' }

# Never overwrite target/release or touch the NVIDIA CUDA cache.
foreach ($name in @('onnxruntime.dll', 'onnxruntime_providers_cuda.dll', 'onnxruntime_providers_shared.dll')) {
    Copy-Item -LiteralPath (Join-Path $ort.DirectoryName $name) -Destination (Join-Path $trial $name) -Force
}
Copy-Item -LiteralPath $exe -Destination (Join-Path $trial 'wolfscribe.exe') -Force
$ffmpeg = Join-Path $project 'tools\ffmpeg.exe'
if (Test-Path -LiteralPath $ffmpeg -PathType Leaf) {
    Copy-Item -LiteralPath $ffmpeg -Destination (Join-Path $trial 'ffmpeg.exe') -Force
}
Remove-Item -LiteralPath $extract -Recurse -Force

Write-Host ''
Write-Host 'Official ONNX Runtime 1.30 CUDA trial prepared!' -ForegroundColor Green
Write-Host 'Original target\release was not modified.'
Write-Host 'Run the isolated test (from the repository root):'
Write-Host '  .\target\blackwell-test\wolfscribe.exe "C:\Projects\ParakeetBench\wolfborn-60s.wav" --device cuda --offline'
Write-Host ''
Write-Host 'The Rust bindings target ORT 1.28; 1.30 compatibility is not yet proven.'
