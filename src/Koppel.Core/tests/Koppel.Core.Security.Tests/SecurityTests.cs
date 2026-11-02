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
    public void Pinned_fingerprint_matches_regardless_of_format()
    {
        using var certificate = ServerCertificate.Load(ServerCertificate.CreatePfx("lab-rack-3"));
        var fingerprint = ServerCertificate.Fingerprint(certificate);

        ServerCertificate.Matches(certificate, fingerprint).Should().BeTrue();
        ServerCertificate.Matches(certificate, fingerprint.Replace(":", "", StringComparison.Ordinal).ToLowerInvariant()).Should().BeTrue();
        using var other = ServerCertificate.Load(ServerCertificate.CreatePfx("lab-rack-3"));
        ServerCertificate.Matches(other, fingerprint).Should().BeFalse();
    }

    [Fact]
    public async Task Encrypted_config_file_round_trips_and_is_unreadable_without_the_key_ring()
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
            await file.StartAsync(ct);
            file.Current.Value.Should().Be("start");
            file.Stop();
        }
    }

    private sealed record Sample(string Value);
}
