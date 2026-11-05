<#
.SYNOPSIS
Writes THIRD-PARTY-NOTICES.txt for what Koppel ships: NuGet packages, npm packages in the UI, and the .NET runtime.

.DESCRIPTION
NuGet: every package in the host's project.assets.json (run `dotnet restore -r win-x64` first), with its
licence and copyright from its .nuspec. npm: the production dependencies of the UI, from `pnpm licenses`
(run `pnpm install --frozen-lockfile` first), with each package's own LICENSE file. .NET: the licence and
third-party notices of the .NET and ASP.NET Core runtime packs, because the Windows installer publishes
self-contained and the container image's runtime carries the same ones.

A package licensed under an SPDX expression but shipping no licence file is covered by the standard text in
licenses\. A licence with no text there stops the script, so a new one is noticed and not skipped.

The result is committed as src\Koppel\src\Koppel\THIRD-PARTY-NOTICES.txt and published with the host, so it
is in the MSI and in the Docker image. build.ps1 regenerates it, then adds the Windows-only notices.

.EXAMPLE
.\new-notices.ps1 -Output ..\src\Koppel\THIRD-PARTY-NOTICES.txt
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Output
)

$ErrorActionPreference = 'Stop'
$installer = $PSScriptRoot
$product = Split-Path $installer
$repo = Split-Path (Split-Path $product)
$assetsPath = Join-Path $product 'src\Koppel\obj\project.assets.json'
$uiFolder = Join-Path $product 'src\Koppel.UI'
$nl = "`n"

if (-not (Test-Path $assetsPath)) { throw "$assetsPath is missing. Run: dotnet restore src\Koppel\src\Koppel\Koppel.csproj -r win-x64" }

function Get-StandardText([string]$spdx) {
    $file = Join-Path $installer "licenses\$spdx.txt"
    if (-not (Test-Path $file)) { throw "No standard text for the licence '$spdx'. Add licenses\$spdx.txt (from https://github.com/spdx/license-list-data)." }
    (Get-Content $file -Raw).Trim()
}

function Get-LicenseFile([string]$folder) {
    Get-ChildItem $folder -File -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -match '^(LICEN[CS]E|COPYING)([.-].*)?$' } |
        Sort-Object Name | Select-Object -First 1
}

# Packages whose .nuspec names no licence. Common.Logging's repository is Apache-2.0 (github.com/net-commons/common-logging).
$declared = @{ 'Common.Logging' = 'Apache-2.0'; 'Common.Logging.Core' = 'Apache-2.0' }

# Entries: Source, Name, Version, License (SPDX id), Copyright, Url, Text (null when the standard text applies).
$entries = New-Object System.Collections.Generic.List[object]

# ---- NuGet ----
$assets = Get-Content $assetsPath -Raw | ConvertFrom-Json
$roots = @($assets.packageFolders.PSObject.Properties.Name)
foreach ($lib in $assets.libraries.PSObject.Properties) {
    if ($lib.Value.type -ne 'package') { continue }
    $id, $version = $lib.Name -split '/'
    $folder = $roots | ForEach-Object { Join-Path $_ (Join-Path $id.ToLowerInvariant() $version) } | Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not $folder) { throw "$($lib.Name) is not in any NuGet folder. Restore first." }
    $nuspec = [xml](Get-Content (Join-Path $folder "$($id.ToLowerInvariant()).nuspec") -Raw)
    $meta = $nuspec.package.metadata
    $license = $null; $text = $null
    if ($meta.license) {
        if ($meta.license.type -eq 'file') { $license = 'see text'; $text = (Get-Content (Join-Path $folder $meta.license.'#text') -Raw).Trim() }
        else { $license = $meta.license.'#text' }
    }
    elseif ($declared.ContainsKey($id)) { $license = $declared[$id] }
    elseif ($meta.licenseUrl -match 'apache\.org/licenses/LICENSE-2\.0') { $license = 'Apache-2.0' }
    elseif ($meta.licenseUrl -match 'opensource\.org/licenses/MIT|licenses\.nuget\.org/MIT') { $license = 'MIT' }
    else {
        $file = Get-LicenseFile $folder
        if ($file) { $license = 'see text'; $text = (Get-Content $file.FullName -Raw).Trim() }
        else { throw "$($lib.Name) declares no licence this script understands (licenseUrl: $($meta.licenseUrl)). Handle it here." }
    }
    $entries.Add([pscustomobject]@{ Source = 'NuGet'; Name = $id; Version = $version; License = $license; Copyright = [string]$meta.copyright; Url = [string]$meta.projectUrl; Text = $text })
}

# ---- npm (what the UI bundles) ----
Push-Location $uiFolder
try {
    $json = (& pnpm licenses list --prod --json) -join "`n"
    if ($LASTEXITCODE -ne 0) { throw 'pnpm licenses failed. Run pnpm install --frozen-lockfile first.' }
}
finally { Pop-Location }
foreach ($group in ($json | ConvertFrom-Json).PSObject.Properties) {
    foreach ($pkg in $group.Value) {
        $file = Get-LicenseFile $pkg.paths[0]
        $text = if ($file) { (Get-Content $file.FullName -Raw).Trim() } else { $null }
        $entries.Add([pscustomobject]@{ Source = 'npm'; Name = $pkg.name; Version = ($pkg.versions -join ', '); License = $group.Name; Copyright = [string]$pkg.author; Url = [string]$pkg.homepage; Text = $text })
    }
}

# ---- Write ----
$out = New-Object System.Collections.Generic.List[string]
$out.Add('Koppel includes the following third-party software.')
$out.Add('')
$out.Add('== Koppel ==')
$out.Add('')
# LICENSE holds the bare GPL text, so the copyright and the or-later grant come first.
$out.Add(@'
Koppel
Copyright (C) 2026 Matumi Software

This program is free software: you can redistribute it and/or modify it under the terms of the GNU
General Public License as published by the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without even
the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the GNU General Public
License for more details.

Source code: https://github.com/matumi-software/koppel
'@)
$out.Add('')
$out.Add((Get-Content (Join-Path $repo 'LICENSE') -Raw).Trim())
$out.Add('')

foreach ($source in 'NuGet', 'npm') {
    $out.Add("== $source packages ==")
    $out.Add('')
    foreach ($e in $entries | Where-Object Source -eq $source | Sort-Object Name) {
        $line = "$($e.Name) $($e.Version): $($e.License)"
        if ($e.Copyright) { $line += "; $($e.Copyright)" }
        if ($e.Url) { $line += "; $($e.Url)" }
        $out.Add($line)
    }
    $out.Add('')
}

# Licence texts shipped by packages, each once, with the packages it covers.
$out.Add('== Licence texts ==')
$out.Add('')
foreach ($group in $entries | Where-Object Text | Group-Object { $_.Text }) {
    $out.Add(('Applies to: ' + (($group.Group | ForEach-Object { "$($_.Name) $($_.Version)" } | Sort-Object -Unique) -join ', ')))
    $out.Add('')
    $out.Add($group.Name)
    $out.Add('')
    $out.Add('----')
    $out.Add('')
}
# Standard texts for the rest. Copyright holders are the ones named above.
$spdx = $entries | Where-Object { -not $_.Text } | ForEach-Object { $_.License -split '\s+(?:OR|AND)\s+' | ForEach-Object { $_.Trim('()') } } | Sort-Object -Unique
foreach ($id in $spdx) {
    $out.Add("$id (applies to the packages above that name it; their copyright holders are listed there)")
    $out.Add('')
    $out.Add((Get-StandardText $id))
    $out.Add('')
    $out.Add('----')
    $out.Add('')
}

# ---- .NET runtime ----
$out.Add('== .NET and ASP.NET Core runtime ==')
$out.Add('')
$out.Add("The Windows installer ships the runtime with Koppel; the container image uses the same one from Microsoft's aspnet image.")
$out.Add('https://github.com/dotnet/runtime, https://github.com/dotnet/aspnetcore')
$out.Add('')
foreach ($pack in 'Microsoft.NETCore.App.Runtime.win-x64', 'Microsoft.AspNetCore.App.Runtime.win-x64') {
    $dep = @($assets.project.frameworks.PSObject.Properties.Value.downloadDependencies) | Where-Object name -eq $pack | Select-Object -First 1
    if (-not $dep) { throw "$pack is not in the assets file. Restore with -r win-x64 first." }
    $version = (($dep.version -replace '[\[\]\(\)\s]', '') -split ',')[0]
    $folder = $roots | ForEach-Object { Join-Path $_ (Join-Path $pack.ToLowerInvariant() $version) } | Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not $folder) { throw "$pack $version is not in any NuGet folder. Restore first." }
    $out.Add("-- $pack ${version}: licence --")
    $out.Add('')
    $out.Add((Get-Content (Get-LicenseFile $folder).FullName -Raw).Trim())
    $out.Add('')
    $thirdParty = Get-ChildItem $folder -File | Where-Object { $_.Name -ieq 'THIRD-PARTY-NOTICES.TXT' } | Select-Object -First 1
    if ($thirdParty) {
        $out.Add("-- $pack ${version}: third-party notices --")
        $out.Add('')
        $out.Add((Get-Content $thirdParty.FullName -Raw).Trim())
        $out.Add('')
    }
}

[IO.File]::WriteAllText($Output, (($out -join $nl) -replace "`r`n", $nl) + $nl, (New-Object Text.UTF8Encoding($false)))
Write-Host "Wrote $Output ($($entries.Count) packages, $([math]::Round((Get-Item $Output).Length / 1KB)) KB)"
