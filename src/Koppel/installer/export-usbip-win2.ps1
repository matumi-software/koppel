<#
.SYNOPSIS
Copies usbip-win2's signed driver package and tools into this folder, for Koppel's Windows installer.

.DESCRIPTION
usbip-win2's installer is built with Inno Setup 7, which no extraction tool reads yet. So the files
come from a machine where usbip-win2 is installed: Windows exports the driver packages unchanged,
with their Microsoft-trusted signatures, and the tools are copied from its install folder.

Run in an administrator PowerShell on a Windows 10 or 11 machine with usbip-win2 installed:
    .\export-usbip-win2.ps1
The files land in .\usbip-win2\<version>\. Commit them, then uninstall usbip-win2 if you like.

usbip-win2 is BSD-2-Clause, (c) Vadym Hrynchyshyn. Its licence goes with the files.
#>
[CmdletBinding()]
param(
    [string]$InstallDir = (Join-Path $env:ProgramFiles 'USBip')
)

$ErrorActionPreference = 'Stop'

$usbip = Join-Path $InstallDir 'usbip.exe'
if (-not (Test-Path $usbip)) {
    throw "usbip-win2 is not installed ($usbip is missing). Install it from https://github.com/vadimgrn/usbip-win2/releases first."
}

$version = (& $usbip --version).Trim()
$target = Join-Path $PSScriptRoot "usbip-win2\$version"
New-Item -ItemType Directory -Force -Path $target | Out-Null

# Find the published names (oemNN.inf) of usbip-win2's two driver packages.
$drivers = @{}
$current = @{}
foreach ($line in (pnputil /enum-drivers)) {
    if ($line -match '^\s*Published Name:\s*(\S+)') { $current = @{ Published = $Matches[1] } }
    elseif ($line -match '^\s*Original Name:\s*(\S+)') { $current.Original = $Matches[1] }
    if ($current.Original -in 'usbip2_ude.inf', 'usbip2_filter.inf') { $drivers[$current.Original] = $current.Published }
}

foreach ($name in 'usbip2_ude.inf', 'usbip2_filter.inf') {
    if (-not $drivers[$name]) { throw "The driver store has no $name. Reinstall usbip-win2." }
    $folder = Join-Path $target ([IO.Path]::GetFileNameWithoutExtension($name))
    New-Item -ItemType Directory -Force -Path $folder | Out-Null
    pnputil /export-driver $drivers[$name] $folder | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "pnputil could not export $name ($($drivers[$name]))." }
}

$tools = Join-Path $target 'tools'
New-Item -ItemType Directory -Force -Path $tools | Out-Null
Copy-Item (Join-Path $InstallDir 'usbip.exe'), (Join-Path $InstallDir 'devnode.exe') -Destination $tools
Get-ChildItem $InstallDir -Filter *.dll | Copy-Item -Destination $tools

Invoke-WebRequest "https://raw.githubusercontent.com/vadimgrn/usbip-win2/v.$version/LICENSE.txt" -OutFile (Join-Path $target 'LICENSE.txt')
Invoke-WebRequest "https://raw.githubusercontent.com/vadimgrn/usbip-win2/v.$version/userspace/CLI11/CLI11.txt" -OutFile (Join-Path $target 'CLI11-LICENSE.txt')

# .inf files are covered by the package's .cat, so only the signed kinds are listed.
Get-ChildItem $target -Recurse -File -Include *.cat, *.sys, *.exe, *.dll | ForEach-Object {
    $signature = Get-AuthenticodeSignature $_.FullName
    [pscustomobject]@{ File = $_.FullName.Substring($target.Length + 1); Signature = $signature.Status }
} | Format-Table -AutoSize

"Exported usbip-win2 $version to $target"
