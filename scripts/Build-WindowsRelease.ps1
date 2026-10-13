#Requires -Version 7.0
<#
Package a versioned portable Windows ZIP.
Bundles SHA-256-verified official ONNX Runtime 1.30 CUDA13 DLLs.
FFmpeg remains separately licensed/installed (WinGet dependency).
#>
[CmdletBinding()]
param()
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$project = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$cargo = Get-Content (Join-Path $project 'Cargo.toml') -Raw
$match = [regex]::Match($cargo, '(?m)^version\s*=\s*"([^"]+)"')
if (-not $match.Success) { throw 'Missing Cargo.toml version.' }
$version = $match.Groups[1].Value
$exe = Join-Path $project 'target\release\wolfscribe.exe'
if (-not (Test-Path $exe -PathType Leaf)) { throw 'Build using cargo build --locked --release --features cuda first.' }

$archiveName = 'onnxruntime-win-x64-gpu_cuda13-1.30.0.zip'
$url = "https://github.com/microsoft/onnxruntime/releases/download/v1.30.0/$archiveName"
$expectedSize = [long]294386823
$expectedSha256 = '8FA4B08359AF682CD605892CB59077049700B640128BB93FD2C7776CF9F55BDC'
$cache = Join-Path $project 'target\release-downloads'
$archive = Join-Path $cache $archiveName
New-Item -ItemType Directory -Force -Path $cache | Out-Null
$verified = (Test-Path $archive -PathType Leaf) -and
    (Get-Item $archive).Length -eq $expectedSize -and
    (Get-FileHash $archive -Algorithm SHA256).Hash -eq $expectedSha256
if (-not $verified) {
    Remove-Item $archive -Force -ErrorAction SilentlyContinue
    Write-Host 'Downloading official Microsoft ONNX Runtime 1.30.0 (294 MB)...'
    & curl.exe --fail --location --retry 3 --retry-all-errors --connect-timeout 30 --proto '=https' --proto-redir '=https' --output $archive $url
    if ($LASTEXITCODE -ne 0) { throw "ONNX download failed with curl exit code $LASTEXITCODE" }
    if ((Get-Item $archive).Length -ne $expectedSize -or (Get-FileHash $archive -Algorithm SHA256).Hash -ne $expectedSha256) {
        throw 'Official ONNX Runtime ZIP does not match the pinned SHA-256 and size.'
    }
}

$dist = Join-Path $project 'target\dist'
$stage = Join-Path $dist 'staging'
$extract = Join-Path $dist 'onnx-extracted'
foreach ($path in @($stage, $extract)) {
    if (Test-Path $path) { Remove-Item $path -Recurse -Force }
    New-Item -ItemType Directory -Path $path -Force | Out-Null
}
Expand-Archive -LiteralPath $archive -DestinationPath $extract -Force
$ort = Get-ChildItem -LiteralPath $extract -Recurse -File -Filter 'onnxruntime.dll' |
    Where-Object {
        (Test-Path (Join-Path $_.DirectoryName 'onnxruntime_providers_cuda.dll')) -and
        (Test-Path (Join-Path $_.DirectoryName 'onnxruntime_providers_shared.dll'))
    } | Select-Object -First 1
if ($null -eq $ort) { throw 'Matching ONNX Runtime DLL trio not found in the official archive.' }

$runtime = Join-Path $stage 'runtime'
New-Item -ItemType Directory -Force -Path $runtime | Out-Null
foreach ($dll in @('onnxruntime.dll', 'onnxruntime_providers_cuda.dll', 'onnxruntime_providers_shared.dll')) {
    Copy-Item (Join-Path $ort.DirectoryName $dll) (Join-Path $runtime $dll) -Force
}
Copy-Item $exe (Join-Path $stage 'wolfscribe.exe') -Force
foreach ($name in @('README.md', 'THIRD_PARTY_NOTICES.md', 'LICENSE')) {
    $original = Join-Path $project $name
    if (Test-Path $original -PathType Leaf) {
        Copy-Item $original (Join-Path $stage $name) -Force
    }
}
# Ship Microsoft ONNX Runtime's own MIT license alongside its DLLs.
$licenseUrl = 'https://raw.githubusercontent.com/microsoft/onnxruntime/v1.30.0/LICENSE'
$licensePath = Join-Path $runtime 'LICENSE-ONNXRUNTIME.txt'
& curl.exe --fail --location --retry 3 --proto '=https' --proto-redir '=https' --output $licensePath $licenseUrl
if ($LASTEXITCODE -ne 0 -or -not (Test-Path $licensePath -PathType Leaf)) {
    throw 'Could not download ONNX Runtime upstream license notice.'
}

$notice = @'
WolfScribe CLI Windows x64 portable package.
This ZIP includes official Microsoft ONNX Runtime 1.30.0 CUDA13 binaries.
The NVIDIA Parakeet model and NVIDIA CUDA/cuDNN runtime are NOT bundled.
Their applicable terms are shown when WolfScribe offers to download them.

FFmpeg is NOT included in this ZIP. Install the independent FFmpeg Essentials
WinGet package: winget install --id Gyan.FFmpeg.Essentials -e
Alternatively, install ffmpeg yourself and add it to PATH.
See THIRD_PARTY_NOTICES.md for licensing and attribution.
'@
Set-Content -LiteralPath (Join-Path $stage 'PACKAGE-NOTES.txt') -Value $notice -Encoding utf8
$releaseZip = Join-Path $dist "WolfScribe-CLI-$version-windows-x64.zip"
if (Test-Path $releaseZip) { Remove-Item $releaseZip -Force }
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $releaseZip -CompressionLevel Optimal
Write-Host "Release ZIP: $releaseZip"
Write-Host "SHA256: $((Get-FileHash $releaseZip -Algorithm SHA256).Hash)"
