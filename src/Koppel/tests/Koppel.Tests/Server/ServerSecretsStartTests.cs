using System.Text;
using Koppel.Core.Configuration;
using Koppel.Core.Security;

namespace Koppel.Tests.Server;

public sealed class ServerSecretsStartTests : IDisposable
{
    private readonly KoppelFactory _koppel = new();

    public void Dispose() => _koppel.Dispose();

    [Fact]
    public void Host_fails_to_start_on_secrets_encrypted_under_another_key_ring_and_leaves_the_file_alone()
    {
        var secretsFile = _koppel.ConfigFile("secrets.json");
        using (var otherKeys = new SecretBox(Path.Combine(_koppel.Directory, "other-keys"), "Koppel.Secrets.v1"))
        {
            File.WriteAllBytes(secretsFile, otherKeys.Protect(Encoding.UTF8.GetBytes("""{ "clients": [] }""")));
        }

        var before = File.ReadAllBytes(secretsFile);

        var start = () => _koppel.Services;

        var failure = start.Should().Throw<Exception>().Which;
        Flatten(failure).Should().ContainSingle(ex => ex is ConfigFileException)
            .Which.Message.Should().Contain("secrets.json").And.Contain("key ring");
        File.ReadAllBytes(secretsFile).Should().Equal(before);
    }

    private static IEnumerable<Exception> Flatten(Exception exception)
    {
        for (var current = exception; current is not null; current = current.InnerException)
        {
            yield return current;
        }
    }
}
