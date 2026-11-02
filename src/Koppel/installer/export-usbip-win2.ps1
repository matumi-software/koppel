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

It also writes SHA256SUMS beside the files. build.ps1 refuses to build if a file no longer matches it, or
if a signed file is not signed by the expected publisher. The sums are a record of what was exported, not
proof that it is what upstream ships: usbip-win2's installer is built with Inno Setup 7, which nothing can
unpack to compare file by file. What can be compared is recorded in SHA256SUMS: the hash and signer of the
official release installer, and the licences against the upstream tags.

.\export-usbip-win2.ps1 -SumsOnly [-Version 0.9.8.1] rewrites SHA256SUMS for files that are already here (no
administrator, no installed usbip-win2), keeping the comment lines.
#>
[CmdletBinding()]
param(
    [string]$InstallDir = (Join-Path $env:ProgramFiles 'USBip'),
    [switch]$SumsOnly,
    [string]$Version,
    # The CLI11 release that usbip-win2's CLI11/CLI11.hpp says it was generated from ("from: v2.7.2").
    # Check that header again when the usbip-win2 version changes.
    [string]$Cli11Tag = 'v2.7.2'
)

$ErrorActionPreference = 'Stop'

function Write-Sums([string]$folder, [string[]]$header) {
    $lines = Get-ChildItem $folder -Recurse -File | Where-Object Name -ne 'SHA256SUMS' | Sort-Object { $_.FullName.Substring($folder.Length) } | ForEach-Object {
        $relative = $_.FullName.Substring($folder.Length).TrimStart('\').Replace('\', '/')
        '{0} *{1}' -f (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant(), $relative
    }
    # LF endings, no BOM: the file is checked in as binary text and read by build.ps1 on any machine.
    [IO.File]::WriteAllText((Join-Path $folder 'SHA256SUMS'), ((@($header) + $lines) -join "`n") + "`n", (New-Object Text.UTF8Encoding($false)))
}

if ($SumsOnly) {
    if (-not $Version) { $Version = (Get-ChildItem (Join-Path $PSScriptRoot 'usbip-win2') -Directory | Sort-Object { [version]$_.Name } | Select-Object -Last 1).Name }
    $folder = Join-Path $PSScriptRoot "usbip-win2\$Version"
    $existing = Join-Path $folder 'SHA256SUMS'
    $keep = if (Test-Path $existing) { @(Get-Content $existing | Where-Object { $_ -match '^#' }) } else { @() }
    Write-Sums $folder $keep
    "Wrote $existing"
    return
}

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
# usbip.exe compiles in CLI11 (BSD-3-Clause). usbip-win2's CLI11.txt is a note, not the licence. The licence is
# taken from the CLI11 tag that matches the compiled-in header, not from main.
Invoke-WebRequest "https://raw.githubusercontent.com/CLIUtils/CLI11/$Cli11Tag/LICENSE" -OutFile (Join-Path $target 'CLI11-LICENSE.txt')

# .inf files are covered by the package's .cat, so only the signed kinds are listed.
Get-ChildItem $target -Recurse -File -Include *.cat, *.sys, *.exe, *.dll | ForEach-Object {
    $signature = Get-AuthenticodeSignature $_.FullName
    [pscustomobject]@{ File = $_.FullName.Substring($target.Length + 1); Signature = $signature.Status }
} | Format-Table -AutoSize

# The official release installer: its hash and signer go in the sums file as the only thing here that comes
# straight from upstream. It is not unpacked (see above).
$installerName = "USBip-$version-x64.exe"
$download = Join-Path ([IO.Path]::GetTempPath()) ([guid]::NewGuid().ToString('N') + '.exe')
Invoke-WebRequest "https://github.com/vadimgrn/usbip-win2/releases/download/v.$version/$installerName" -OutFile $download -UseBasicParsing
try {
    $installerSignature = Get-AuthenticodeSignature $download
    $installerSigner = ($installerSignature.SignerCertificate.Subject -split ', ' | Where-Object { $_ -like 'CN=*' } | Select-Object -First 1)
    $header = @(
        "# usbip-win2 ${version}: files exported from an installed copy by export-usbip-win2.ps1."
        "# Official release asset $installerName, sha256 $((Get-FileHash $download -Algorithm SHA256).Hash.ToLowerInvariant()), Authenticode $($installerSignature.Status), $installerSigner"
        "# https://github.com/vadimgrn/usbip-win2/releases/tag/v.$version"
        "# LICENSE.txt is from that tag; CLI11-LICENSE.txt is from CLIUtils/CLI11 tag $Cli11Tag."
    )
}
finally { Remove-Item $download -Force -ErrorAction SilentlyContinue }
Write-Sums $target $header

"Exported usbip-win2 $version to $target"
