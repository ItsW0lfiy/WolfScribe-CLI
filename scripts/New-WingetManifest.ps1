#Requires -Version 7.0
<#
Generate the 3 official WinGet community repository manifests from the actual
published release archive, once the project owner selects a source license.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$ZipPath,
    [Parameter(Mandatory)][string]$Version,
    [Parameter(Mandatory)][string]$License,
    [string]$Tag,
    [string]$OutputDirectory = './target/winget-manifests'
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $Tag) { $Tag = "v$Version" }
if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw 'Expected numeric version like 0.1.0.' }
if ($License -notmatch '^[A-Za-z0-9.+\- ]+$') { throw 'Expected an approved SPDX license identifier.' }

$archive = (Resolve-Path -LiteralPath $ZipPath).Path
$hash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash
$folder = Join-Path $OutputDirectory "manifests\i\ItsW0lfiy\WolfScribe\$Version"
New-Item -ItemType Directory -Force -Path $folder | Out-Null

$identifier = 'ItsW0lfiy.WolfScribe'
$yamlSchema = '$schema'
$versionManifest = @"
# yaml-language-server: $yamlSchema=https://aka.ms/winget-manifest.version.1.12.0.schema.json
PackageIdentifier: $identifier
PackageVersion: $Version
DefaultLocale: en-US
ManifestType: version
ManifestVersion: 1.12.0
"@
$url = "https://github.com/ItsW0lfiy/WolfScribe-CLI/releases/download/$Tag/WolfScribe-CLI-$Version-windows-x64.zip"
$installerManifest = @"
# yaml-language-server: $yamlSchema=https://aka.ms/winget-manifest.installer.1.12.0.schema.json
PackageIdentifier: $identifier
PackageVersion: $Version
InstallerType: zip
NestedInstallerType: portable
NestedInstallerFiles:
  - RelativeFilePath: wolfscribe.exe
    PortableCommandAlias: wolfscribe
UpgradeBehavior: uninstallPrevious
Dependencies:
  PackageDependencies:
    - PackageIdentifier: Gyan.FFmpeg.Essentials
Installers:
  - Architecture: x64
    InstallerUrl: $url
    InstallerSha256: $hash
ManifestType: installer
ManifestVersion: 1.12.0
"@
$localeManifest = @"
# yaml-language-server: $yamlSchema=https://aka.ms/winget-manifest.defaultLocale.1.12.0.schema.json
PackageIdentifier: $identifier
PackageVersion: $Version
PackageLocale: en-US
Publisher: ItsW0lfiy
PublisherUrl: https://github.com/ItsW0lfiy
PackageName: WolfScribe CLI
PackageUrl: https://github.com/ItsW0lfiy/WolfScribe-CLI
License: $License
LicenseUrl: https://github.com/ItsW0lfiy/WolfScribe-CLI/blob/$Tag/LICENSE
ShortDescription: Local transcription of audio and video into SRT and TXT using Parakeet TDT.
Description: WolfScribe is a local CLI transcriber with batch input and optional NVIDIA CUDA acceleration.
Moniker: wolfscribe
Tags:
  - transcription
  - subtitles
  - audio
  - offline
  - cuda
ManifestType: defaultLocale
ManifestVersion: 1.12.0
"@
Set-Content -LiteralPath (Join-Path $folder "$identifier.yaml") -Value $versionManifest -Encoding utf8
Set-Content -LiteralPath (Join-Path $folder "$identifier.installer.yaml") -Value $installerManifest -Encoding utf8
Set-Content -LiteralPath (Join-Path $folder "$identifier.locale.en-US.yaml") -Value $localeManifest -Encoding utf8
Write-Host "Generated WinGet manifests at: $folder"
Write-Host "Release ZIP SHA256: $hash"
Write-Host 'Validate with winget validate --manifest <folder>; test installation and submit as a PR to microsoft/winget-pkgs.'
