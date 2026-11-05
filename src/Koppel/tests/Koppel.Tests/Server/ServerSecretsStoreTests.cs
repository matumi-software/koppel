using System.Net;
using System.Security.Cryptography;
using System.Security.Cryptography.X509Certificates;
using System.Text.Json;
using Koppel.Core.Security;
using Koppel.Server;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Logging.Abstractions;
using Skakel.Protocol;
using Skakel.Server;

namespace Koppel.Tests.Server;

/// <summary>
/// <c>secrets.json</c> as Skakel's store: a round trip, a first start, and a file Koppel 0.1.0 wrote, which
/// is carried over when it can be and kept aside when it cannot.
/// </summary>
public sealed class ServerSecretsStoreTests : IDisposable
{
    private const string Sha256Client = "client-0a1b2c3d4e";
    private const string Pbkdf2Client = "client-9f8e7d6c5b";

    private readonly string _directory = Path.Combine(Path.GetTempPath(), "koppel-tests", Guid.NewGuid().ToString("N"));
    private readonly SecretBox _box;

    public ServerSecretsStoreTests()
    {
        _box = new SecretBox(Path.Combine(_directory, "keys"), "test");
        Directory.CreateDirectory(Path.Combine(_directory, "config"));
    }

    private KoppelPaths Paths => new(Path.Combine(_directory, "config"), Path.Combine(_directory, "keys"));

    public void Dispose()
    {
        _box.Dispose();
        if (Directory.Exists(_directory))
        {
            Directory.Delete(_directory, recursive: true);
        }
    }

    [Fact]
    public async Task LoadAsync_is_null_on_the_first_start_and_reads_back_what_was_saved()
    {
        var ct = TestContext.Current.CancellationToken;
        using (var first = NewStore())
        {
            (await first.LoadAsync(ct)).Should().BeNull();
            await first.SaveAsync(State(), ct);
        }

        using var again = NewStore();
        var loaded = await again.LoadAsync(ct);

        loaded!.Clients.Should().ContainSingle().Which.Id.Should().Be(Sha256Client);
        loaded.SigningKey.KeyId.Should().Be("skakel-1");
        File.ReadAllText(Paths.ServerSecretsFile).Should().NotContain(Sha256Client, "the file is encrypted");
    }

    [Fact]
    public async Task LoadAsync_carries_a_0_1_0_file_over_and_drops_clients_with_an_older_hash()
    {
        var ct = TestContext.Current.CancellationToken;
        var (pfx, key) = (NewPfx(), NewKey());
        WriteLegacy(pfx, key);

        SkakelState? carried;
        using (var store = NewStore())
        {
            carried = await store.LoadAsync(ct);
        }

        carried!.CertificatePfx.Should().Be(pfx, "clients keep the fingerprint they pinned");
        carried.SigningKey.Should().Be(new SkakelKeyRecord("koppel-1", key, new DateTimeOffset(2026, 11, 3, 0, 0, 0, TimeSpan.Zero)));
        carried.Clients.Should().ContainSingle().Which.Should().Be(new SkakelClientRecord(Sha256Client, "bench-1", Secrets.Hash("kept-secret"), new DateTimeOffset(2026, 11, 2, 0, 0, 0, TimeSpan.Zero), 3, "kept-secret"));
        carried.NextCertificate!.ReceivedBy.Should().Equal(Sha256Client);
        using var again = NewStore();
        (await again.LoadAsync(ct)).Should().BeEquivalentTo(carried, "the carried-over state was saved in the new form");
    }

    [Fact]
    public async Task LoadAsync_starts_fresh_from_a_0_1_0_file_it_cannot_use_and_keeps_it_aside()
    {
        var ct = TestContext.Current.CancellationToken;
        WriteLegacy(NewPfx(), "not a key");
        var before = File.ReadAllBytes(Paths.ServerSecretsFile);
        using var store = NewStore();

        var loaded = await store.LoadAsync(ct);

        loaded.Should().BeNull("Skakel then makes a new certificate and signing key");
        Directory.GetFiles(Paths.ConfigDirectory, "secrets.json.0.1.0-*").Should().ContainSingle()
            .Which.Should().Match(aside => File.ReadAllBytes(aside).SequenceEqual(before));
    }

    [Fact]
    public async Task A_server_upgraded_from_0_1_0_keeps_its_certificate_and_its_clients_get_tokens()
    {
        var ct = TestContext.Current.CancellationToken;
        using var koppel = new KoppelFactory();
        var pfx = NewPfx();
        using (var hostBox = new SecretBox(Path.Combine(koppel.Directory, "keys"), "Koppel.Secrets.v1"))
        {
            File.WriteAllBytes(koppel.ConfigFile("secrets.json"), hostBox.AsFileCodec().Encode(LegacyJson(pfx, NewKey())));
        }

        using var certificate = X509CertificateLoader.LoadPkcs12(Convert.FromBase64String(pfx), password: null);
        koppel.Services.GetRequiredService<SkakelKeys>().Fingerprint.Should().Be(Fingerprints.Of(certificate));
        using var https = koppel.CreateHttpsClient();
        using var token = await https.PostAsync("/skakel/v1/token", new FormUrlEncodedContent(new Dictionary<string, string>
        {
            ["grant_type"] = "client_credentials",
            ["client_id"] = Sha256Client,
            ["client_secret"] = "kept-secret",
        }), ct);
        token.StatusCode.Should().Be(HttpStatusCode.OK);
    }

    private ServerSecretsStore NewStore() => new(Paths, _box, TimeProvider.System, NullLogger<ServerSecretsStore>.Instance);

    private void WriteLegacy(string pfx, string key) =>
        File.WriteAllBytes(Paths.ServerSecretsFile, _box.AsFileCodec().Encode(LegacyJson(pfx, key)));

    /// <summary>What Koppel 0.1.0 wrote: a client with a SHA-256 hash, one with a PBKDF2 hash, a retired key and a prepared certificate.</summary>
    private static string LegacyJson(string pfx, string key) => JsonSerializer.Serialize(new
    {
        signingKey = key,
        certificatePfx = pfx,
        signingKeySince = (DateTimeOffset?)new DateTimeOffset(2026, 11, 3, 0, 0, 0, TimeSpan.Zero),
        clients = new object[]
        {
            new { id = Sha256Client, name = "bench-1", secretHash = Secrets.Hash("kept-secret"), createdAt = new DateTimeOffset(2026, 11, 2, 0, 0, 0, TimeSpan.Zero), secret = "kept-secret", generation = 3 },
            new { id = Pbkdf2Client, name = "bench-old", secretHash = "pbkdf2-sha256$210000$c2FsdA==$aGFzaA==", createdAt = new DateTimeOffset(2026, 10, 1, 0, 0, 0, TimeSpan.Zero), secret = (string?)null, generation = 1 },
        },
        retiredSigningKeys = new[] { new { key = NewKey(), keyId = "koppel-0", acceptUntil = DateTimeOffset.UtcNow.AddMinutes(10) } },
        nextCertificate = new { pfx = NewPfx(), preparedAt = DateTimeOffset.UtcNow, receivedBy = new[] { Sha256Client, Pbkdf2Client } },
    });

    private static SkakelState State() => new()
    {
        Clients = [new SkakelClientRecord(Sha256Client, "bench-1", Secrets.Hash("kept-secret"), DateTimeOffset.UtcNow, 1, "kept-secret")],
        SigningKey = new SkakelKeyRecord("skakel-1", NewKey(), DateTimeOffset.UtcNow),
        CertificatePfx = NewPfx(),
    };

    private static string NewKey()
    {
        using var rsa = RSA.Create(2048);
        return Convert.ToBase64String(rsa.ExportPkcs8PrivateKey());
    }

    private static string NewPfx()
    {
        using var key = ECDsa.Create(ECCurve.NamedCurves.nistP256);
        var request = new CertificateRequest("CN=lab-rack-3, O=Koppel", key, HashAlgorithmName.SHA256);
        using var certificate = request.CreateSelfSigned(DateTimeOffset.UtcNow.AddDays(-1), DateTimeOffset.UtcNow.AddYears(10));
        return Convert.ToBase64String(certificate.Export(X509ContentType.Pfx));
    }
}
