<#
.SYNOPSIS
Creates Koppel's data folder (%ProgramData%\Koppel) so only SYSTEM and administrators can use it, or checks that it is still like that.

.DESCRIPTION
%ProgramData% lets any standard user create folders in it. Koppel keeps its key ring and config
there and its service runs as LocalSystem, so a folder a user created first (owned by them, or a
junction into somewhere else) would let that user read the keys or steer what SYSTEM writes.

Without -Check this refuses to use a folder that is a reparse point or is owned by anyone other than
SYSTEM, Administrators or TrustedInstaller, and fails with a message saying what to do. It does
not take such a folder over: whoever owned it may still hold open handles or have planted files in
it, and resetting the ACL would not undo that. A missing folder is created, and a folder that
passes has its owner set to Administrators and its ACL replaced with a protected one that grants
only SYSTEM and Administrators full control.

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

function Test-Folder {
    $item = Get-Item -LiteralPath $Path -Force
    if (-not $item.PSIsContainer) { throw "$Path is a file. Koppel needs a folder there. Move the file away and run setup again." }
    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw "$Path is a junction or symbolic link. Koppel keeps its keys there and will not follow it. Delete the link and run setup again."
    }
    $owner = (Get-Acl -LiteralPath $Path).GetOwner([Security.Principal.SecurityIdentifier])
    if ($owner.Value -notin $trusted) {
        $name = try { $owner.Translate([Security.Principal.NTAccount]).Value } catch { $owner.Value }
        throw "$Path is owned by $name, not by Administrators. Koppel keeps its keys there and will not use a folder another account controls. Rename or delete the folder (it may hold keys from an earlier install: copy them out first if you need them) and run setup again."
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

$acl = New-Object Security.AccessControl.DirectorySecurity
$acl.SetOwner($administrators)
$acl.SetAccessRuleProtection($true, $false)
foreach ($sid in $system, $administrators) {
    $acl.AddAccessRule((New-Object Security.AccessControl.FileSystemAccessRule($sid, 'FullControl', 'ContainerInherit, ObjectInherit', 'None', 'Allow')))
}
Set-Acl -LiteralPath $Path -AclObject $acl
Test-Folder
Write-Output "$Path is owned by Administrators and open to SYSTEM and Administrators only."
