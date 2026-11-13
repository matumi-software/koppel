<#
.SYNOPSIS
Installs or removes the usbip-win2 driver that Koppel attaches USB devices with on Windows.

.DESCRIPTION
Koppel's installer runs this as LocalSystem from its install folder (usbip\driver.ps1). It does what
usbip-win2's own installer does: adds the upper filter driver, creates the virtual USB host
controller, and registers a task that detaches attached devices when Windows shuts down.

It records which driver packages and which device node its own install created, in
%ProgramData%\Koppel\driver-state.json, and -Uninstall removes only those. A usbip-win2 that was
installed on its own is detected, left alone and used as it is.

Everything goes to standard output, which the MSI log captures. It writes no log file on purpose:
%ProgramData% is a place a standard user can pre-create files in, and this runs as SYSTEM.
Exit codes: 0 done, anything else failed. WixQuietExec turns 3010 (restart needed) into a failure,
so a needed restart, after an install or an uninstall, is left in HKLM\SOFTWARE\Matumi\Koppel (DriverRestartRequired) for the MSI to
read; see Koppel.Installer/restart-flag.js.
#>
[CmdletBinding()]
param(
    [switch]$Install,
    [switch]$Uninstall
)

$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
$hardwareId = 'ROOT\USBIP_WIN2\UDE'
$taskName = 'Koppel detach USB devices on shutdown'
$pnputil = Join-Path $env:SystemRoot 'System32\pnputil.exe'
$statePath = Join-Path $env:ProgramData 'Koppel\driver-state.json'
$driverNames = @('usbip2_ude.inf', 'usbip2_filter.inf')
$rebootRequired = $false
$restartKey = 'HKLM:\SOFTWARE\Matumi\Koppel'

function Get-Inf([string]$name) {
    Get-ChildItem (Join-Path $here "drivers\$name") -Filter "$name.inf" -Recurse | Select-Object -First 1 -ExpandProperty FullName
}

function Invoke-Tool([string]$exe, [string[]]$arguments, [int[]]$ok = @(0)) {
    Write-Output "> $exe $arguments"
    & $exe @arguments
    if ($LASTEXITCODE -notin $ok) { throw "$exe $arguments exited with $LASTEXITCODE" }
    # 3010: done, a restart finishes it. It is handed to the MSI through the registry at the end.
    if ($LASTEXITCODE -eq 3010) { $script:rebootRequired = $true }
}

function Get-Controller {
    Get-CimInstance Win32_PnPEntity | Where-Object { $_.HardwareID -contains $hardwareId }
}

function Get-DriverStorePackages {
    # Every package in the driver store as @{ Published = 'oemNN.inf'; Original = 'usbip2_ude.inf' }. Windows
    # names a package oemNN.inf once it is stored, and reuses the number after a package is deleted. Read
    # without parsing English: pnputil's XML output first (Windows 11 and newer builds of 10), then DISM's.
    $text = & $pnputil /enum-drivers /format xml | Out-String
    if ($text.TrimStart().StartsWith('<')) {
        try {
            $found = @(([xml]$text).PnpUtil.Driver | ForEach-Object { @{ Published = [string]$_.DriverName; Original = [string]$_.OriginalName } })
            if ($found.Count -gt 0) { return $found }
        }
        catch { Write-Host "pnputil's XML could not be read ($($_.Exception.Message)); asking Windows another way." }
    }
    else { Write-Host 'This pnputil has no XML output; asking Windows another way.' }
    try {
        @(Get-WindowsDriver -Online -All -ErrorAction Stop | ForEach-Object {
            @{ Published = [string]$_.Driver; Original = [IO.Path]::GetFileName([string]$_.OriginalFileName) }
        })
    }
    catch {
        # Last resort, and only right on an English Windows.
        Write-Host "Get-WindowsDriver failed ($_); reading pnputil's text output."
        $current = @{}
        foreach ($line in (& $pnputil /enum-drivers)) {
            if ($line -match '^\s*Published Name:\s*(\S+)') { $current = @{ Published = $Matches[1] } }
            elseif ($line -match '^\s*Original Name:\s*(\S+)') { $current.Original = $Matches[1]; $current }
        }
    }
}

function Get-PublishedDriver([string[]]$names) {
    Get-DriverStorePackages | Where-Object { $_.Original -in $names } | ForEach-Object { $_.Published }
}

function Select-OwnPackages([string[]]$published) {
    # Of the recorded names, the ones Windows still lists as one of usbip-win2's own packages. A number
    # can be handed to some other driver after Koppel's package left the store, and that one is not ours to delete.
    $listed = @(Get-DriverStorePackages | Where-Object { $_.Original -like 'usbip2_*.inf' } | ForEach-Object { $_.Published })
    foreach ($name in $published) {
        if (-not (Test-PackageName $name)) { Write-Host "Skipping '$name': it is not a driver package name." }
        elseif ($name -notin $listed) { Write-Host "Skipping ${name}: Windows no longer lists it as a usbip-win2 package (already removed, or the number was reused)." }
        else { $name }
    }
}

function Get-StandaloneUsbip {
    # usbip-win2's own installer puts it in Program Files\USBip and lists it in Apps as USBip. Koppel's
    # copy lives in its own folder and has no Apps entry, so this never mistakes it for that.
    $folder = Join-Path $env:ProgramFiles 'USBip'
    if (Test-Path (Join-Path $folder 'usbip.exe')) { return $folder }
    $keys = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall', 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall'
    Get-ChildItem $keys -ErrorAction SilentlyContinue | ForEach-Object { Get-ItemProperty $_.PSPath } |
        Where-Object { $_.DisplayName -like 'USBip*' -or $_.DisplayName -like 'usbip-win2*' } |
        Select-Object -First 1 -ExpandProperty DisplayName
}

function Test-DataFolder {
    # Write-Host, not Write-Output: the result of this function is the boolean, a message in the output would make it always true.
    # The state file is read and written only in a folder that `koppel prepare-data-folder --check` accepts. It says why
    # on standard error when it does not, which the MSI log captures; the exit code is the answer.
    & (Join-Path (Split-Path $here) 'Koppel.exe') prepare-data-folder --check | Out-Null
    if ($LASTEXITCODE -eq 0) { return $true }
    Write-Host 'The data folder is not safe to use (see the line above).'
    $false
}

function Test-StateFile {
    # The state file steers what a SYSTEM process deletes, so it is trusted only if an administrator owns it,
    # it is not a link, and nobody else can write to it or change who may.
    $item = Get-Item -LiteralPath $statePath -Force
    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { return $false }
    $trusted = @('S-1-5-18', 'S-1-5-32-544', 'S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464')
    $acl = Get-Acl -LiteralPath $statePath
    if ($acl.GetOwner([Security.Principal.SecurityIdentifier]).Value -notin $trusted) { return $false }
    $write = [Security.AccessControl.FileSystemRights]'WriteData, AppendData, WriteExtendedAttributes, WriteAttributes, Delete, ChangePermissions, TakeOwnership'
    foreach ($rule in $acl.GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier])) {
        if ($rule.AccessControlType -eq 'Allow' -and ($rule.FileSystemRights -band $write) -and $rule.IdentityReference.Value -notin $trusted) { return $false }
    }
    $true
}

function Test-PackageName([string]$name) {
    $name -match '^oem\d+\.inf\z'
}

function Read-State {
    # $null when there is no file. Throws if there is one that cannot be trusted.
    if (-not (Test-Path $statePath)) { return }
    if (-not (Test-StateFile)) { throw "$statePath is not owned and protected by administrators." }
    Get-Content $statePath -Raw | ConvertFrom-Json
}

function Write-State([string[]]$packages, [bool]$deviceNode) {
    [ordered]@{ packages = @($packages); deviceNode = $deviceNode } | ConvertTo-Json | Set-Content -Path $statePath -Encoding utf8
}

function Register-ShutdownTask {
    $query = "<QueryList><Query Id=`"0`" Path=`"System`"><Select Path=`"System`">*[System[Provider[@Name='Microsoft-Windows-Kernel-Power'] and (EventID=109)]] or *[System[Provider[@Name='User32'] and (EventID=1074)]]</Select></Query></QueryList>"
    $usbip = [Security.SecurityElement]::Escape((Join-Path $here 'usbip.exe'))
    $xml = @"
<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Author>Koppel</Author>
    <Description>Detaches USB devices attached from other machines when Windows shuts down or restarts.</Description>
  </RegistrationInfo>
  <Triggers>
    <EventTrigger>
      <Enabled>true</Enabled>
      <Subscription>$([Security.SecurityElement]::Escape($query))</Subscription>
    </EventTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>S-1-5-19</UserId>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <ExecutionTimeLimit>PT30S</ExecutionTimeLimit>
    <Priority>7</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>"$usbip"</Command>
      <Arguments>detach --all=closeonly</Arguments>
    </Exec>
  </Actions>
</Task>
"@
    Register-ScheduledTask -TaskName $taskName -Xml $xml -Force | Out-Null
    Write-Output "Registered the task '$taskName'."
}

function Install-Driver {
    if (-not (Test-DataFolder)) { throw 'The data folder check failed, so the install cannot record what it adds.' }
    # A standalone usbip-win2 is looked for first, whatever the state file says: the first standalone run writes
    # an empty record, and a later upgrade or repair must not put Koppel's packages in over it.
    $standalone = Get-StandaloneUsbip
    if ($standalone) {
        # Not Koppel's: leave its driver and device node exactly as they are, and never remove them.
        Write-Output "Found a usbip-win2 that was installed on its own ($standalone). Koppel will use its driver and device node and will not change or remove them."
        Write-Output 'If that usbip-win2 is a different version from the one Koppel ships, Koppel may not work with it. Uninstalling it removes the driver Koppel relies on.'
        if (-not (Test-Path $statePath)) { Write-State @() $false }
        return
    }

    try { $state = Read-State }
    catch {
        Write-Output "WARNING: ignoring the record of an earlier install: $_"
        Remove-Item $statePath -Force
        $state = $null
    }
    if ($state -and @(@($state.packages) | Where-Object { -not (Test-PackageName $_) }).Count) {
        Write-Output 'WARNING: ignoring the record of an earlier install: it lists a name that is not a driver package.'
        $state = $null
    }

    $before = @(Get-PublishedDriver $driverNames)
    $controllerBefore = [bool](Get-Controller)
    if ($state) {
        # An upgrade or a repair: keep what the first install recorded.
        $known = @(Select-OwnPackages @($state.packages))
        $node = [bool]$state.deviceNode
    }
    else {
        # No record and no standalone usbip-win2: either a first install, or a Koppel from before records
        # were kept. Packages and a node that are already here are taken to be that Koppel's.
        if ($before.Count -gt 0 -or $controllerBefore) { Write-Output 'Existing usbip-win2 driver packages or device node found with no record; treating them as an earlier Koppel install.' }
        $known = $before
        $node = $controllerBefore
    }

    # Recorded after each step, so a failure half way still leaves a record for the rollback to remove by.
    $record = {
        $added = @(Get-PublishedDriver $driverNames | Where-Object { $_ -notin $before })
        Write-Output "Driver packages this install added: $($added -join ', ')"
        Write-State @(@($known) + $added | Select-Object -Unique) $node
    }

    # 259: the package is already in the driver store and no device needed it, as on an upgrade to the same driver.
    Invoke-Tool $pnputil @('/add-driver', (Get-Inf 'usbip2_filter'), '/install') @(0, 3010, 259)
    & $record
    if ($controllerBefore) {
        Write-Output 'The virtual USB host controller already exists; updating its driver.'
        Invoke-Tool $pnputil @('/add-driver', (Get-Inf 'usbip2_ude'), '/install') @(0, 3010, 259)
    }
    else {
        $node = $true
        Invoke-Tool (Join-Path $here 'devnode.exe') @('install', (Get-Inf 'usbip2_ude'), $hardwareId)
    }
    & $record
    Register-ShutdownTask
}

function Uninstall-Driver {
    # Removal carries on past failures, so a half-installed driver can still be cleaned up, and every
    # failure is written out so the MSI log shows it.
    $failed = $false
    try { Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction Stop }
    catch { Write-Output "The task '$taskName' was not removed: $_" }

    $standalone = Get-StandaloneUsbip
    if ($standalone) {
        Write-Output "Found a usbip-win2 that was installed on its own ($standalone). Its driver and device node are left in place."
        return $true
    }
    if (-not (Test-DataFolder)) {
        Write-Output 'WARNING: the record of what Koppel installed cannot be trusted, so the driver was left in place. Remove it with pnputil /delete-driver if you want it gone.'
        return $false
    }
    try { $state = Read-State }
    catch {
        Write-Output "WARNING: the record of what Koppel installed cannot be trusted ($_), so the driver was left in place. Remove it with pnputil /delete-driver if you want it gone."
        return $false
    }
    if (-not $state) {
        Write-Output 'WARNING: there is no record of what Koppel installed (an install from before records were kept), so its driver packages and device node were left in place.'
        return $false
    }

    if ($state.deviceNode) {
        & (Join-Path $here 'devnode.exe') remove $hardwareId root
        Write-Output "devnode remove exited with $LASTEXITCODE"
        if ($LASTEXITCODE -ne 0) { $failed = $true; Write-Output "WARNING: the device node $hardwareId was not removed." }
    }
    foreach ($published in @(Select-OwnPackages @($state.packages))) {
        & $pnputil /delete-driver $published /uninstall /force
        Write-Output "pnputil /delete-driver $published exited with $LASTEXITCODE"
        if ($LASTEXITCODE -notin 0, 3010) { $failed = $true; Write-Output "WARNING: the driver package $published was not removed. Remove it with: pnputil /delete-driver $published /uninstall /force" }
        elseif ($LASTEXITCODE -eq 3010) { $script:rebootRequired = $true }
    }
    if (-not $failed) { Remove-Item $statePath -Force -ErrorAction SilentlyContinue }
    -not $failed
}

function Save-RestartFlag {
    # pnputil said 3010. The MSI asks for the restart, so the install or uninstall itself still succeeds.
    if (-not $rebootRequired) { return }
    Write-Output 'A restart is needed to finish. Asking the installer for one.'
    New-Item -Path $restartKey -Force | Out-Null
    New-ItemProperty -Path $restartKey -Name DriverRestartRequired -Value 1 -PropertyType DWord -Force | Out-Null
}

try {
    if ($Install) {
        Remove-ItemProperty -Path $restartKey -Name DriverRestartRequired -ErrorAction SilentlyContinue
        Install-Driver
    }
    if ($Uninstall) {
        Remove-Item -Path $restartKey -Recurse -ErrorAction SilentlyContinue
        if (-not (Uninstall-Driver)) { Save-RestartFlag; exit 1 }
    }
}
catch {
    Write-Output "Failed: $_"
    exit 1
}

Save-RestartFlag
exit 0
