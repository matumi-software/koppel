<#
.SYNOPSIS
Installs or removes the usbip-win2 driver that Koppel attaches USB devices with on Windows.

.DESCRIPTION
Koppel's installer runs this as LocalSystem from its install folder (usbip\driver.ps1). It does what
usbip-win2's own installer does: adds the upper filter driver, creates the virtual USB host
controller, and registers a task that detaches attached devices when Windows shuts down.
Output goes to %ProgramData%\Koppel\logs\driver.log.
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

$logs = Join-Path $env:ProgramData 'Koppel\logs'
New-Item -ItemType Directory -Force -Path $logs | Out-Null
Start-Transcript -Path (Join-Path $logs 'driver.log') -Append | Out-Null

function Get-Inf([string]$name) {
    Get-ChildItem (Join-Path $here "drivers\$name") -Filter "$name.inf" -Recurse | Select-Object -First 1 -ExpandProperty FullName
}

function Invoke-Tool([string]$exe, [string[]]$arguments, [int[]]$ok = @(0)) {
    Write-Output "> $exe $arguments"
    & $exe @arguments
    if ($LASTEXITCODE -notin $ok) { throw "$exe $arguments exited with $LASTEXITCODE" }
}

function Get-Controller {
    Get-CimInstance Win32_PnPEntity | Where-Object { $_.HardwareID -contains $hardwareId }
}

function Get-PublishedDriver([string[]]$names) {
    # pnputil names packages oemNN.inf once they are in the driver store.
    $current = @{}
    foreach ($line in (& $pnputil /enum-drivers)) {
        if ($line -match '^\s*Published Name:\s*(\S+)') { $current = @{ Published = $Matches[1] } }
        elseif ($line -match '^\s*Original Name:\s*(\S+)' -and $Matches[1] -in $names) { $current.Published }
    }
}

try {
    if ($Install) {
        # 3010: installed, a restart finishes it.
        Invoke-Tool $pnputil @('/add-driver', (Get-Inf 'usbip2_filter'), '/install') @(0, 3010)
        if (Get-Controller) {
            Write-Output 'The virtual USB host controller already exists; updating its driver.'
            Invoke-Tool $pnputil @('/add-driver', (Get-Inf 'usbip2_ude'), '/install') @(0, 3010)
        }
        else {
            Invoke-Tool (Join-Path $here 'devnode.exe') @('install', (Get-Inf 'usbip2_ude'), $hardwareId)
        }

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

    if ($Uninstall) {
        # Removal carries on past failures, so a half-installed driver can still be cleaned up.
        Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction SilentlyContinue
        & (Join-Path $here 'devnode.exe') remove $hardwareId root
        Write-Output "devnode remove exited with $LASTEXITCODE"
        foreach ($published in (Get-PublishedDriver @('usbip2_ude.inf', 'usbip2_filter.inf'))) {
            & $pnputil /delete-driver $published /uninstall /force
            Write-Output "pnputil /delete-driver $published exited with $LASTEXITCODE"
        }
    }
}
catch {
    Write-Output "Failed: $_"
    Stop-Transcript | Out-Null
    exit 1
}

Stop-Transcript | Out-Null
exit 0
