<#
.SYNOPSIS
Creates Koppel's data folder (%ProgramData%\Koppel) so only SYSTEM and administrators can use it, or checks that it is still like that.

.DESCRIPTION
%ProgramData% lets any standard user create folders in it. Koppel keeps its key ring and config
there and its service runs as LocalSystem, so a folder a standard user created first (owned by them,
or a junction into somewhere else) would let that user read the keys or steer what SYSTEM writes.

Without -Check this refuses to use a folder that is a file, a reparse point, or owned by an account
that is not an administrator, and fails with a message saying what to do. An administrator is SYSTEM,
Administrators, TrustedInstaller, or an account that is a member of the local Administrators group,
directly or through a group it belongs to. The last case matters: on many machines whatever an
administrator creates is owned by that administrator, so an earlier install may have left the folder
owned by one. Such a folder is taken over: its owner is set to Administrators and its ACL, and the ACL
of everything under it, is replaced with a protected one that grants only SYSTEM and Administrators
full control. A link found under it is deleted (the link only, never what it points to). A missing
folder is created the same way.

With -Check it only verifies, and never changes anything.

Koppel's installer runs this as LocalSystem from its install folder (usbip\data-folder.ps1).
#>
[CmdletBinding()]
param(
    [string]$Path = (Join-Path $env:ProgramData 'Koppel'),
    [switch]$Check
)

$ErrorActionPreference = 'Stop'

$system = [Security.Principal.SecurityIdentifier]'S-1-5-18'
$administrators = [Security.Principal.SecurityIdentifier]'S-1-5-32-544'
$trustedInstaller = [Security.Principal.SecurityIdentifier]'S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464'
$trusted = @($system.Value, $administrators.Value, $trustedInstaller.Value)

function Get-AdministratorSids {
    # The accounts and groups directly in the local Administrators group. ADSI lists AzureAD and domain
    # members that Get-LocalGroupMember trips over.
    $group = [ADSI]'WinNT://./Administrators,group'
    foreach ($member in $group.Invoke('Members')) {
        $bytes = [byte[]]([ADSI]$member).InvokeGet('objectSid')
        (New-Object Security.Principal.SecurityIdentifier($bytes, 0)).Value
    }
}

function Test-Administrator([Security.Principal.SecurityIdentifier]$sid) {
    if ($sid.Value -in $trusted) { return $true }
    $members = @(Get-AdministratorSids)
    if ($sid.Value -in $members) { return $true }
    # A member through a group: ask Windows for the account's groups (works for domain accounts and for
    # local ones that are signed in; failing to resolve means "not shown to be an administrator").
    try {
        $account = $sid.Translate([Security.Principal.NTAccount]).Value
        $identity = New-Object Security.Principal.WindowsIdentity(($account -replace '^[^\\]*\\', '') + '@' + ($account -replace '\\.*$', ''))
        foreach ($group in $identity.Groups) { if ($group.Value -in $members -or $group.Value -eq $administrators.Value) { return $true } }
    }
    catch { }
    $false
}

function Test-Folder {
    $item = Get-Item -LiteralPath $Path -Force
    if (-not $item.PSIsContainer) { throw "$Path is a file. Koppel needs a folder there. Move the file away and run setup again." }
    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw "$Path is a junction or symbolic link. Koppel keeps its keys there and will not follow it. Delete the link and run setup again."
    }
    $owner = (Get-Acl -LiteralPath $Path).GetOwner([Security.Principal.SecurityIdentifier])
    if (-not (Test-Administrator $owner)) {
        $name = try { $owner.Translate([Security.Principal.NTAccount]).Value } catch { $owner.Value }
        throw "$Path is owned by $name, who is not an administrator. Koppel keeps its keys there and will not use a folder a standard user controls. Rename or delete the folder (it may hold keys from an earlier install: copy them out first if you need them) and run setup again."
    }
}

function Protect-Item([string]$path, [bool]$container) {
    $acl = if ($container) { New-Object Security.AccessControl.DirectorySecurity } else { New-Object Security.AccessControl.FileSecurity }
    $acl.SetOwner($administrators)
    $acl.SetAccessRuleProtection($true, $false)
    $inherit = if ($container) { 'ContainerInherit, ObjectInherit' } else { 'None' }
    foreach ($sid in $system, $administrators) {
        $acl.AddAccessRule((New-Object Security.AccessControl.FileSystemAccessRule($sid, 'FullControl', $inherit, 'None', 'Allow')))
    }
    Set-Acl -LiteralPath $path -AclObject $acl
}

function Protect-Tree([string]$folder) {
    Protect-Item $folder $true
    foreach ($child in Get-ChildItem -LiteralPath $folder -Force) {
        if ($child.Attributes -band [IO.FileAttributes]::ReparsePoint) {
            # Deleting a link removes the link and not its target. Never follow it.
            Write-Output "Removing the link $($child.FullName) (it points somewhere else; Koppel does not use links)."
            if ($child.PSIsContainer) { [IO.Directory]::Delete($child.FullName) } else { [IO.File]::Delete($child.FullName) }
        }
        elseif ($child.PSIsContainer) { Protect-Tree $child.FullName }
        else { Protect-Item $child.FullName $false }
    }
}

$created = $false
if (-not (Test-Path -LiteralPath $Path)) {
    if ($Check) { throw "$Path does not exist." }
    # Without -Force this fails if something made the folder in the meantime, so a success means it is ours.
    New-Item -ItemType Directory -Path $Path | Out-Null
    $created = $true
}

if (-not $created) { Test-Folder }
if ($Check) { return }

Protect-Tree $Path
Test-Folder
Write-Output "$Path is owned by Administrators and open to SYSTEM and Administrators only."
