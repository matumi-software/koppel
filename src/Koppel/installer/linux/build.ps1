<#
.SYNOPSIS
Builds Koppel's Linux packages: bin\koppel_<version>_amd64.deb and bin\koppel_<version>_arm64.deb.

.DESCRIPTION
Runs the linux-packages target of src\Koppel\src\Koppel\Dockerfile, so it needs Docker and nothing else. That
target uses the image's pinned toolchain, restores from the lock files, publishes Koppel self-contained and
ReadyToRun for linux-x64 and linux-arm64, and packages each with nfpm and the files in this folder. The
version is <Version> from src\Directory.Build.props.

Run from anywhere: .\build.ps1
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$linux = $PSScriptRoot
$src = Split-Path (Split-Path (Split-Path $linux))
$dockerfile = Join-Path $src 'Koppel\src\Koppel\Dockerfile'
$out = Join-Path $linux 'bin'

# docker writes its progress to stderr, which Windows PowerShell turns into errors when it is redirected; under
# 'Stop' the first line would end the script. The exit code decides instead.
& {
    $ErrorActionPreference = 'Continue'
    docker buildx build -f $dockerfile --target linux-packages --output "type=local,dest=$out" $src
}
if ($LASTEXITCODE -ne 0) { throw "docker buildx build failed with exit code $LASTEXITCODE." }

Get-ChildItem $out -Filter *.deb | ForEach-Object { Write-Host "Built $($_.FullName)" }
