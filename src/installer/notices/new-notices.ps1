<#
.SYNOPSIS
Writes THIRD-PARTY-NOTICES.txt for what Koppel ships: the Rust crates in its programs and the npm packages the
UI bundles.

.DESCRIPTION
Rust: every crate in the programs for the -Target triples, from cargo-about (cargo install cargo-about --locked
--features cli), checked against the licences about.toml accepts, with the licence texts cargo-about finds in
each crate. npm: the production dependencies of the UI, from `pnpm licenses` (run `pnpm install
--frozen-lockfile` first), with each package's own LICENSE file. An npm package licensed under an SPDX
expression but shipping no licence file is covered by the standard text in licenses\. A licence with no text
there stops the script, so a new one is noticed and not skipped.

The Windows installer's build.ps1 runs it for x86_64-pc-windows-msvc (Koppel.exe and KoppelDesktop.exe), writes it into
the staging folder, not the source tree, then adds the Windows-only notices. The Linux build
(src\koppel\Dockerfile) runs it under PowerShell on Linux for the .deb files and the image. There cargo
and pnpm live in different stages, so the Dockerfile runs cargo-about with the same about.toml and passes
the JSON in with -AboutJson. It also passes Debian's copy of the GPL with -License, since the build sees
only src\ and not the repo's LICENSE (the two are the same text).

This folder is shared by both builds: the script, about.toml and the standard licence texts in licenses\.

.EXAMPLE
.\new-notices.ps1 -Output ..\windows\obj\staging\THIRD-PARTY-NOTICES.txt
pwsh new-notices.ps1 -Target x86_64-unknown-linux-musl -Output /tmp/THIRD-PARTY-NOTICES.txt
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Output,
    # The targets whose crates are listed. Several are merged into one list.
    [string[]]$Target = @('x86_64-pc-windows-msvc'),
    # cargo-about's JSON, already written, instead of running cargo-about here.
    [string]$AboutJson,
    # Koppel's licence text, the repo's LICENSE by default.
    [string]$License
)

$ErrorActionPreference = 'Stop'
$notices = $PSScriptRoot
# src\installer\notices: two folders up is src\, three the repo.
$src = Split-Path (Split-Path $notices)
$repo = Split-Path $src
if (-not $License) { $License = Join-Path $repo 'LICENSE' }
# Join-Path one folder at a time, so the paths work under PowerShell on Linux too.
$uiFolder = Join-Path (Join-Path $src 'ui') 'app'
$nl = "`n"

function Get-StandardText([string]$spdx) {
    $file = Join-Path (Join-Path $notices 'licenses') "$spdx.txt"
    if (-not (Test-Path $file)) { throw "No standard text for the licence '$spdx'. Add licenses\$spdx.txt (from https://github.com/spdx/license-list-data)." }
    (Get-Content $file -Raw).Trim()
}

function Get-LicenseFile([string]$folder) {
    Get-ChildItem $folder -File -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -match '^(LICEN[CS]E|COPYING)([.-].*)?$' } |
        Sort-Object Name | Select-Object -First 1
}

# Entries: Source, Name, Version, License (SPDX id), Copyright, Url, Text (null when the standard text applies).
$entries = New-Object System.Collections.Generic.List[object]

# ---- Rust crates (cargo-about) ----
$rustTexts = @()
if ($AboutJson) {
    $report = Get-Content $AboutJson -Raw -Encoding utf8 | ConvertFrom-Json
}
else {
    $about = Join-Path ([IO.Path]::GetTempPath()) "koppel-about-$PID.json"
    $targets = $Target | ForEach-Object { '--target'; $_ }
    & cargo about generate --format json -c (Join-Path $notices 'about.toml') -m (Join-Path (Join-Path $src 'koppel') 'Cargo.toml') @targets -o $about
    if ($LASTEXITCODE -ne 0) { throw 'cargo about failed. Install it with: cargo install cargo-about --locked --features cli. A licence about.toml does not accept stops it too.' }
    $report = Get-Content $about -Raw -Encoding utf8 | ConvertFrom-Json
    Remove-Item $about
}
foreach ($crate in $report.crates) {
    $package = $crate.package
    # Koppel's own crates are covered by Koppel's licence above.
    if ($package.name -like 'koppel*') { continue }
    $entries.Add([pscustomobject]@{ Source = 'Rust'; Name = $package.name; Version = $package.version; License = [string]$crate.license; Copyright = (@($package.authors) -join ', '); Url = [string]$package.repository; Text = $null; Covered = $true })
}
# Each distinct text once, with the crates it covers.
foreach ($group in @($report.licenses) | Group-Object { $_.text.Trim() }) {
    $users = $group.Group | ForEach-Object { $_.used_by } | ForEach-Object { $_.crate } | Where-Object { $_.name -notlike 'koppel*' } | ForEach-Object { "$($_.name) $($_.version)" } | Sort-Object -Unique
    if ($users) { $rustTexts += [pscustomobject]@{ Id = $group.Group[0].id; Users = $users; Text = $group.Name } }
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
$out.Add((Get-Content $License -Raw).Trim())
$out.Add('')

foreach ($source in 'Rust', 'npm') {
    $out.Add($(if ($source -eq 'Rust') { '== Rust crates ==' } else { "== $source packages ==" }))
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
foreach ($t in $rustTexts) {
    $out.Add("$($t.Id), applies to: " + ($t.Users -join ', '))
    $out.Add('')
    $out.Add($t.Text)
    $out.Add('')
    $out.Add('----')
    $out.Add('')
}
# Standard texts for the rest. Copyright holders are the ones named above.
$spdx = $entries | Where-Object { -not $_.Text -and -not $_.Covered } | ForEach-Object { $_.License -split '\s+(?:OR|AND)\s+' | ForEach-Object { $_.Trim('()') } } | Sort-Object -Unique
foreach ($id in $spdx) {
    $out.Add("$id (applies to the packages above that name it; their copyright holders are listed there)")
    $out.Add('')
    $out.Add((Get-StandardText $id))
    $out.Add('')
    $out.Add('----')
    $out.Add('')
}

[IO.File]::WriteAllText($Output, (($out -join $nl) -replace "`r`n", $nl) + $nl, (New-Object Text.UTF8Encoding($false)))
Write-Host "Wrote $Output ($($entries.Count) packages, $([math]::Round((Get-Item $Output).Length / 1KB)) KB)"
