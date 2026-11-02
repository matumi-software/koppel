using System.Runtime.Versioning;
using System.Security.AccessControl;
using System.Security.Principal;

namespace Koppel.Core.Security;

/// <summary>
/// Restricts a directory that holds secrets or config (a key ring, an encrypted store). Idempotent, safe
/// to call on every start. Same approach as Matumi.Roots.Security.
/// </summary>
/// <remarks>
/// <para>
/// Windows: the owner becomes BUILTIN\Administrators and the DACL is protected (no inheritance, so the
/// broad Users grant a parent such as <c>%ProgramData%</c> hands down is dropped) with full control for
/// SYSTEM and Administrators only. A directory that is a reparse point (junction or symlink) is refused,
/// because the ACL would land on its target.
/// </para>
/// <para>
/// A process that is not elevated cannot take the owner away from itself, and would lock itself out of
/// the directory, so it also gets full control and the owner is left alone. That is a developer running
/// from a console; the service runs as LocalSystem. POSIX: <c>chmod 700</c>.
/// </para>
/// </remarks>
public static class DirectoryHardening
{
    public static void Harden(string directory)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(directory);

        if (OperatingSystem.IsWindows())
        {
            HardenWindows(directory);
        }
        else
        {
            File.SetUnixFileMode(directory, UnixFileMode.UserRead | UnixFileMode.UserWrite | UnixFileMode.UserExecute);
        }
    }

    [SupportedOSPlatform("windows")]
    private static void HardenWindows(string directory)
    {
        var info = new DirectoryInfo(directory);
        if (info.Attributes.HasFlag(FileAttributes.ReparsePoint) || info.LinkTarget is not null)
        {
            throw new IOException($"{directory} is a junction or symbolic link. Koppel will not keep its files behind a link: point Paths:Config and Paths:Keys at a real directory.");
        }

        var system = new SecurityIdentifier(WellKnownSidType.LocalSystemSid, domainSid: null);
        var administrators = new SecurityIdentifier(WellKnownSidType.BuiltinAdministratorsSid, domainSid: null);

        using var current = WindowsIdentity.GetCurrent();
        var privileged = current.IsSystem || new WindowsPrincipal(current).IsInRole(WindowsBuiltInRole.Administrator);

        var security = new DirectorySecurity();
        security.SetAccessRuleProtection(isProtected: true, preserveInheritance: false);
        AddFullControl(security, system);
        AddFullControl(security, administrators);
        if (!privileged && current.User is { } user)
        {
            AddFullControl(security, user);
        }

        if (privileged)
        {
            security.SetOwner(administrators);
        }

        info.SetAccessControl(security);
    }

    [SupportedOSPlatform("windows")]
    private static void AddFullControl(DirectorySecurity security, IdentityReference identity) =>
        security.AddAccessRule(new FileSystemAccessRule(
            identity,
            FileSystemRights.FullControl,
            InheritanceFlags.ContainerInherit | InheritanceFlags.ObjectInherit,
            PropagationFlags.None,
            AccessControlType.Allow));
}
