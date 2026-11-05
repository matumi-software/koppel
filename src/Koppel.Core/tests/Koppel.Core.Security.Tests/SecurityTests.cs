using System.Text.Json;
using Koppel.Core.Configuration;
using Microsoft.Extensions.Logging.Abstractions;

namespace Koppel.Core.Security.Tests;

public sealed class SecurityTests : IDisposable
{
    private readonly string _directory = Path.Combine(Path.GetTempPath(), "koppel-core-tests", Guid.NewGuid().ToString("N"));

    public void Dispose()
    {
        if (Directory.Exists(_directory))
        {
            Directory.Delete(_directory, recursive: true);
        }
    }

    [Fact]
    public void Hash_verifies_the_right_secret_only()
    {
        var secret = Secrets.NewSecret();
        var hash = Secrets.Hash(secret);

        Secrets.Verify(secret, hash).Should().BeTrue();
        Secrets.Verify(secret + "x", hash).Should().BeFalse();
        Secrets.Verify(secret, "garbage").Should().BeFalse();
        hash.Should().NotContain(secret);
    }

    [Fact]
    public void New_hashes_are_sha256_and_never_contain_the_secret()
    {
        var secret = Secrets.NewSecret();

        var hash = Secrets.Hash(secret);

        hash.Should().StartWith("sha256$").And.NotContain(secret);
        Secrets.Verify(secret, hash).Should().BeTrue();
        Secrets.IsLegacy(hash).Should().BeFalse();
    }

    [Theory]
    [InlineData("sha256$")]
    [InlineData("sha256$not base64!")]
    [InlineData("sha256$AAAA")]
    [InlineData("pbkdf2-sha256$1$AAAA")]
    [InlineData("pbkdf2-sha256$abc$AAAA$AAAA")]
    public void Malformed_hashes_never_verify(string stored)
    {
        Secrets.Verify("secret", stored).Should().BeFalse();
    }

    [Fact]
    public void A_pbkdf2_hash_from_an_older_version_still_verifies()
    {
        const string secret = "an-old-secret";
        var salt = System.Security.Cryptography.RandomNumberGenerator.GetBytes(16);
        var derived = System.Security.Cryptography.Rfc2898DeriveBytes.Pbkdf2(System.Text.Encoding.UTF8.GetBytes(secret), salt, 210_000, System.Security.Cryptography.HashAlgorithmName.SHA256, 32);
        var legacy = $"pbkdf2-sha256$210000${Convert.ToBase64String(salt)}${Convert.ToBase64String(derived)}";

        Secrets.Verify(secret, legacy).Should().BeTrue();
        Secrets.Verify(secret + "x", legacy).Should().BeFalse();
        Secrets.IsLegacy(legacy).Should().BeTrue();
    }

    [Fact]
    public async Task Encrypted_config_file_round_trips_and_is_refused_without_the_key_ring()
    {
        var ct = TestContext.Current.CancellationToken;
        var path = Path.Combine(_directory, "config", "secret.json");
        using (var box = new SecretBox(Path.Combine(_directory, "keys"), "test"))
        using (var file = new JsonConfigFile<Sample>(path, new Sample("start"), JsonSerializerOptions.Web, NullLogger.Instance, box.AsFileCodec()))
        {
            await file.StartAsync(ct);
            await file.UpdateAsync(_ => new Sample("hunter2-and-more"), ct);
            file.Stop();
        }

        (await File.ReadAllTextAsync(path, ct)).Should().NotContain("hunter2");

        using (var box = new SecretBox(Path.Combine(_directory, "keys"), "test"))
        using (var file = new JsonConfigFile<Sample>(path, new Sample("start"), JsonSerializerOptions.Web, NullLogger.Instance, box.AsFileCodec()))
        {
            await file.StartAsync(ct);
            file.Current.Value.Should().Be("hunter2-and-more");
            file.Stop();
        }

        using (var otherKeys = new SecretBox(Path.Combine(_directory, "other-keys"), "test"))
        using (var file = new JsonConfigFile<Sample>(path, new Sample("start"), JsonSerializerOptions.Web, NullLogger.Instance, otherKeys.AsFileCodec()))
        {
            var before = await File.ReadAllBytesAsync(path, ct);

            var start = () => file.StartAsync(ct);

            await start.Should().ThrowAsync<ConfigFileException>();
            (await File.ReadAllBytesAsync(path, ct)).Should().Equal(before);
        }
    }

    private sealed record Sample(string Value);
}
