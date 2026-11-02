using System.Runtime.Versioning;
using System.Security.AccessControl;
using System.Security.Principal;

namespace Koppel.Core.Security;

/// <summary>
/// Restricts a directory that holds secrets (a key ring, an encrypted store). Idempotent, safe to call
/// on every start. Same approach as Matumi.Roots.Security.
/// </summary>
/// <remarks>
/// Windows: a non-inherited ACL with full control for SYSTEM, Administrators and the process identity,
/// so the broad Users grant a parent such as <c>%ProgramData%</c> hands down is dropped.
/// POSIX: <c>chmod 700</c>.
/// </remarks>
public static class DirectoryHardening
{
    public static void Harden(string directory)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(directory);

        if (OperatingSystem.IsWindows())
        {
            HardenWindowsAcl(directory);
        }
        else
        {
            File.SetUnixFileMode(directory, UnixFileMode.UserRead | UnixFileMode.UserWrite | UnixFileMode.UserExecute);
        }
    }

    [SupportedOSPlatform("windows")]
    private static void HardenWindowsAcl(string directory)
    {
        var security = new DirectorySecurity();
        security.SetAccessRuleProtection(isProtected: true, preserveInheritance: false);

        var system = new SecurityIdentifier(WellKnownSidType.LocalSystemSid, domainSid: null);
        var administrators = new SecurityIdentifier(WellKnownSidType.BuiltinAdministratorsSid, domainSid: null);
        AddFullControl(security, system);
        AddFullControl(security, administrators);

        using var current = WindowsIdentity.GetCurrent();
        if (current.User is { } user && user != system && user != administrators)
        {
            AddFullControl(security, user);
        }

        // Ownership is left alone: changing it needs elevation. The DACL is the boundary.
        new DirectoryInfo(directory).SetAccessControl(security);
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
