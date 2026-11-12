<#
.SYNOPSIS
Builds the Rust Koppel's Linux packages, bin\koppel_<version>_{amd64,arm64,armhf}.deb, and on request the
multi-platform image, kept on this machine.

.DESCRIPTION
Runs src\installer\docker\Dockerfile, so it needs Docker and nothing else. The linux-packages target builds the UI,
cross-compiles static binaries for amd64, arm64 and armv6 with cargo-zigbuild, and packages each with nfpm and
the files in this folder. The version is the workspace version in src\Cargo.toml.

-Image also builds the image for linux/amd64, linux/arm64 and linux/arm/v6 and loads it into this machine's
image store as matumisoftware/koppel:<version> (or -Tag). It never pushes. Loading a multi-platform image needs
Docker's containerd image store, the default since Docker Engine 29 and Docker Desktop 4.34.

Run from anywhere: .\build.ps1, or .\build.ps1 -Image
#>
[CmdletBinding()]
param(
    # Also build the image and load it here.
    [switch]$Image,
    # The image's name and tag, matumisoftware/koppel:<version> by default.
    [string]$Tag
)

$ErrorActionPreference = 'Stop'
$linux = $PSScriptRoot
$src = Split-Path (Split-Path $linux)
$dockerfile = Join-Path $src 'installer\docker\Dockerfile'
$out = Join-Path $linux 'bin'

# docker writes its progress to stderr, which Windows PowerShell turns into errors when it is redirected; under
# 'Stop' the first line would end the script. The exit code decides instead (the preference set here holds
# only inside the function). The arguments go in as one array: splatting $args loses -t's value.
function Invoke-Docker([string[]]$Arguments) {
    $ErrorActionPreference = 'Continue'
    docker @Arguments
    if ($LASTEXITCODE -ne 0) { throw "docker $($Arguments[0..1] -join ' ') failed with exit code $LASTEXITCODE." }
}

# The build has no .git, so the commit that `koppel --version` names goes in as a build argument.
$commit = (git -C $src rev-parse --short=9 HEAD 2>$null)
if ($LASTEXITCODE -ne 0) { $commit = '' }

Invoke-Docker @('buildx', 'build', '-f', $dockerfile, '--build-arg', "KOPPEL_GIT_COMMIT=$commit", '--target', 'linux-packages', '--output', "type=local,dest=$out", $src)
$debs = Get-ChildItem $out -Filter 'koppel_*.deb'
$debs | ForEach-Object { Write-Host "Built $($_.FullName)" }

if ($Image) {
    if (-not $Tag) {
        $version = Select-String -Path (Join-Path $src 'Cargo.toml') -Pattern '^version = "(.+)"' |
            Select-Object -First 1 | ForEach-Object { $_.Matches[0].Groups[1].Value }
        if (-not $version) { throw 'No version = "..." found in src\Cargo.toml.' }
        $Tag = "matumisoftware/koppel:$version"
    }
    Invoke-Docker @('buildx', 'build', '-f', $dockerfile, '--build-arg', "KOPPEL_GIT_COMMIT=$commit", '--platform', 'linux/amd64,linux/arm64,linux/arm/v6', '-t', $Tag, '--load', $src)
    Write-Host "Built $Tag for linux/amd64, linux/arm64 and linux/arm/v6 (loaded here, not pushed)"
}
