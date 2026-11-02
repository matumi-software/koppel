<#
.SYNOPSIS
Builds Koppel's Windows installer: bin\Koppel-Setup.exe.

.DESCRIPTION
1. Publishes Koppel self-contained for win-x64, UI included, so the machine needs no .NET, and
   compiled ahead of time (ReadyToRun), so the first requests after a start are not slow.
2. Adds usbip-win2's driver and tools (from usbip-win2\<version>, see export-usbip-win2.ps1),
   driver.ps1 and the third-party notices.
3. Builds the MSI, then setup.exe, which installs the VC++ runtime first.

Run from anywhere: .\build.ps1 [-Version 0.1.0]
#>
[CmdletBinding()]
param(
    [string]$Version = '0.1.0'
)

$ErrorActionPreference = 'Stop'
$installer = $PSScriptRoot
$product = Split-Path $installer
$repo = Split-Path (Split-Path $product)
$staging = Join-Path $installer 'obj\staging'
$cache = Join-Path $installer 'obj\cache'
$out = Join-Path $installer 'bin'

function Invoke-Checked([scriptblock]$command) {
    & $command
    if ($LASTEXITCODE -ne 0) { throw "Failed with exit code $LASTEXITCODE`: $command" }
}

Remove-Item $staging -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $staging, $cache, $out | Out-Null

Write-Host 'Publishing Koppel...'
Invoke-Checked { dotnet publish (Join-Path $product 'src\Koppel\Koppel.csproj') -c Release -r win-x64 --self-contained -o $staging -p:Version=$Version -p:PublishReadyToRun=true -nologo -v q }

$usbipWin2 = Get-ChildItem (Join-Path $installer 'usbip-win2') -Directory | Sort-Object { [version]$_.Name } | Select-Object -Last 1
if (-not $usbipWin2) { throw 'No usbip-win2 files. Run export-usbip-win2.ps1 first.' }
Write-Host "Adding usbip-win2 $($usbipWin2.Name)..."
$usbip = Join-Path $staging 'usbip'
New-Item -ItemType Directory -Force -Path (Join-Path $usbip 'drivers') | Out-Null
Copy-Item (Join-Path $usbipWin2.FullName 'tools\*') $usbip
Copy-Item (Join-Path $usbipWin2.FullName 'usbip2_ude'), (Join-Path $usbipWin2.FullName 'usbip2_filter') (Join-Path $usbip 'drivers') -Recurse
Copy-Item (Join-Path $installer 'driver.ps1') $usbip

$notices = @(
    'Koppel includes the following third-party software.'
    ''
    "== usbip-win2 $($usbipWin2.Name) (driver, usbip.exe, devnode.exe, libusbip.dll) =="
    'https://github.com/vadimgrn/usbip-win2'
    ''
    (Get-Content (Join-Path $usbipWin2.FullName 'LICENSE.txt') -Raw)
    '== CLI11 (compiled into usbip.exe) =='
    'https://github.com/CLIUtils/CLI11'
    ''
    (Get-Content (Join-Path $usbipWin2.FullName 'CLI11-LICENSE.txt') -Raw)
    '== Microsoft Visual C++ Redistributable =='
    'Installed by Koppel-Setup.exe under Microsoft''s own licence terms, which it shows when run on its own.'
    ''
    '== Koppel =='
    ''
    (Get-Content (Join-Path $repo 'LICENSE') -Raw)
)
Set-Content -Path (Join-Path $staging 'THIRD-PARTY-NOTICES.txt') -Value ($notices -join "`r`n") -Encoding utf8

$vcRedist = Join-Path $cache 'vc_redist.x64.exe'
if (-not (Test-Path $vcRedist)) {
    Write-Host 'Downloading the VC++ runtime...'
    Invoke-WebRequest 'https://aka.ms/vs/17/release/vc_redist.x64.exe' -OutFile $vcRedist -UseBasicParsing
}
$signature = Get-AuthenticodeSignature $vcRedist
if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch 'O=Microsoft Corporation') {
    throw "vc_redist.x64.exe is not signed by Microsoft ($($signature.Status)). Delete $vcRedist and try again."
}
$vcRedistBuild = (Get-Item $vcRedist).VersionInfo.FileBuildPart

Write-Host 'Building the MSI...'
$msiOut = Join-Path $installer 'obj\msi'
Invoke-Checked { dotnet build (Join-Path $installer 'Koppel.Installer\Koppel.Installer.wixproj') -c Release -nologo -v q -o $msiOut -p:StagingDir=$staging -p:Version=$Version }

Write-Host 'Building setup.exe...'
Invoke-Checked { dotnet build (Join-Path $installer 'Koppel.Setup\Koppel.Setup.wixproj') -c Release -nologo -v q -o $out -p:Version=$Version -p:Msi=$(Join-Path $msiOut 'Koppel.msi') -p:VCRedist=$vcRedist -p:VCRedistBuild=$vcRedistBuild }

$setup = Join-Path $out 'Koppel-Setup.exe'
Write-Host "Built $setup ($([math]::Round((Get-Item $setup).Length / 1MB, 1)) MB)"
