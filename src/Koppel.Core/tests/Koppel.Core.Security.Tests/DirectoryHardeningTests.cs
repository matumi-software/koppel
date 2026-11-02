using System.Runtime.Versioning;
using System.Security.AccessControl;
using System.Security.Principal;

namespace Koppel.Core.Security.Tests;

public sealed class DirectoryHardeningTests : IDisposable
{
    private readonly string _directory = Path.Combine(Path.GetTempPath(), "koppel-core-tests", Guid.NewGuid().ToString("N"));

    public DirectoryHardeningTests() => Directory.CreateDirectory(_directory);

    public void Dispose()
    {
        if (Directory.Exists(_directory))
        {
            Directory.Delete(_directory, recursive: true);
        }
    }

    [Fact]
    public void Harden_is_idempotent()
    {
        DirectoryHardening.Harden(_directory);
        DirectoryHardening.Harden(_directory);

        File.WriteAllText(Path.Combine(_directory, "probe.txt"), "still writable by the owner");
    }

    [Fact]
    public void Harden_sets_mode_700_on_unix()
    {
        if (OperatingSystem.IsWindows())
        {
            return;
        }

        DirectoryHardening.Harden(_directory);

        File.GetUnixFileMode(_directory).Should().Be(UnixFileMode.UserRead | UnixFileMode.UserWrite | UnixFileMode.UserExecute);
    }

    [Fact]
    public void Harden_drops_inherited_access_and_leaves_no_users_grant_on_windows()
    {
        if (!OperatingSystem.IsWindows())
        {
            return;
        }

        DirectoryHardening.Harden(_directory);

        AssertWindowsAcl();
    }

    [SupportedOSPlatform("windows")]
    private void AssertWindowsAcl()
    {
        var security = new DirectoryInfo(_directory).GetAccessControl();
        var rules = security.GetAccessRules(true, true, typeof(SecurityIdentifier)).Cast<FileSystemAccessRule>().ToList();
        var allowed = rules.Where(rule => rule.AccessControlType == AccessControlType.Allow).Select(rule => (SecurityIdentifier)rule.IdentityReference).ToList();
        security.AreAccessRulesProtected.Should().BeTrue();
        rules.Should().NotContain(rule => rule.IsInherited);
        allowed.Should().Contain(new SecurityIdentifier(WellKnownSidType.LocalSystemSid, null));
        allowed.Should().Contain(new SecurityIdentifier(WellKnownSidType.BuiltinAdministratorsSid, null));
        allowed.Should().NotContain(new SecurityIdentifier(WellKnownSidType.BuiltinUsersSid, null));
        allowed.Should().NotContain(new SecurityIdentifier(WellKnownSidType.WorldSid, null));
    }

    [Fact]
    public void Harden_refuses_a_directory_that_is_a_link_on_windows()
    {
        if (!OperatingSystem.IsWindows())
        {
            return;
        }

        var target = Path.Combine(_directory, "target");
        var link = Path.Combine(_directory, "link");
        Directory.CreateDirectory(target);
        try
        {
            Directory.CreateSymbolicLink(link, target);
        }
        catch (Exception ex) when (ex is UnauthorizedAccessException or IOException)
        {
            return; // Creating a symlink needs a privilege this user may not have.
        }

        var harden = () => DirectoryHardening.Harden(link);

        harden.Should().Throw<IOException>().WithMessage("*junction or symbolic link*");
    }
}
