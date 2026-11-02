using System.Security.Cryptography;
using System.Text;
using Koppel.Core.Configuration;
using Koppel.Core.Security;
using Koppel.Server;
using Microsoft.Extensions.Logging.Abstractions;

namespace Koppel.Tests.Server;

/// <summary>Client secrets are hashed with SHA-256 now; clients registered with the older PBKDF2 hash still work and move to the new one.</summary>
public sealed class SecretHashTests : IDisposable
{
    private readonly string _directory = Path.Combine(Path.GetTempPath(), "koppel-tests", Guid.NewGuid().ToString("N"));
    private readonly SecretBox _box;
    private readonly KoppelPaths _paths;

    public SecretHashTests()
    {
        _box = new SecretBox(Path.Combine(_directory, "keys"), "test");
        _paths = new KoppelPaths(Path.Combine(_directory, "config"), Path.Combine(_directory, "keys"));
    }

    public void Dispose()
    {
        _box.Dispose();
        if (Directory.Exists(_directory))
        {
            Directory.Delete(_directory, recursive: true);
        }
    }

    [Fact]
    public async Task A_new_client_is_stored_with_a_sha256_hash_and_validates()
    {
        var ct = TestContext.Current.CancellationToken;
        using var secrets = await StartAsync(ct);

        var (client, secret) = await secrets.CreateClientAsync("bench", ct);

        (await secrets.ValidateAsync(client.Id, secret, ct)).Should().Be(1);
        (await secrets.ValidateAsync(client.Id, secret + "x", ct)).Should().BeNull();
        File.Exists(_paths.ServerSecretsFile).Should().BeTrue();
    }

    [Fact]
    public async Task A_client_with_an_older_pbkdf2_hash_still_validates_and_is_upgraded()
    {
        var ct = TestContext.Current.CancellationToken;
        const string secret = "legacy-secret";
        await SeedAsync(new RegisteredClient("client-old", "old", Pbkdf2(secret), DateTimeOffset.UnixEpoch), ct);
        using var secrets = await StartAsync(ct);

        (await secrets.ValidateAsync("client-old", "wrong", ct)).Should().BeNull();
        (await secrets.ValidateAsync("client-old", secret, ct)).Should().HaveValue();
        secrets.Dispose();

        using var reloaded = new JsonConfigFile<ServerSecrets>(_paths.ServerSecretsFile, ServerSecrets.Empty, KoppelJson.Files, NullLogger.Instance, _box.AsFileCodec());
        await reloaded.StartAsync(ct);
        var hash = reloaded.Current.Clients.Single().SecretHash;
        hash.Should().StartWith("sha256$");
        Secrets.Verify(secret, hash).Should().BeTrue();
    }

    private async Task<ServerSecretsStore> StartAsync(CancellationToken ct)
    {
        var store = new ServerSecretsStore(_paths, _box, TimeProvider.System, NullLogger<ServerSecretsStore>.Instance);
        await store.StartAsync(ct);
        return store;
    }

    private async Task SeedAsync(RegisteredClient client, CancellationToken ct)
    {
        using var file = new JsonConfigFile<ServerSecrets>(_paths.ServerSecretsFile, ServerSecrets.Empty, KoppelJson.Files, NullLogger.Instance, _box.AsFileCodec());
        await file.StartAsync(ct);
        await file.UpdateAsync(current => current with { Clients = [client] }, ct);
    }

    private static string Pbkdf2(string secret)
    {
        var salt = RandomNumberGenerator.GetBytes(16);
        var hash = Rfc2898DeriveBytes.Pbkdf2(Encoding.UTF8.GetBytes(secret), salt, 210_000, HashAlgorithmName.SHA256, 32);
        return $"pbkdf2-sha256$210000${Convert.ToBase64String(salt)}${Convert.ToBase64String(hash)}";
    }
}
