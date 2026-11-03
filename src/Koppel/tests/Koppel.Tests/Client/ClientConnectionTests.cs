using Koppel.Client;

namespace Koppel.Tests.Client;

/// <summary>The client's side of a certificate handover: what it trusts, what it stores, and when it is still the same server.</summary>
public sealed class ClientConnectionTests
{
    private const string Current = "3F:9A:1C:2B:7D:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA";
    private const string Next = "00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF";
    private const string Other = "AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA";

    private static readonly ClientConnection Saved = new("https://lab-rack-3:5240", Current, "client-0a1b2c3d4e", "secret");

    [Fact]
    public void WithServerCertificates_stores_the_next_fingerprint_the_server_announced()
    {
        var learned = Saved.WithServerCertificates(Current, Next.Replace(":", "", StringComparison.Ordinal).ToLowerInvariant());

        learned.Should().Be(Saved with { NextServerFingerprint = Next });
        learned.Continues(Saved).Should().BeTrue();
    }

    [Fact]
    public void WithServerCertificates_pins_the_next_fingerprint_once_the_server_presents_it()
    {
        var waiting = Saved with { NextServerFingerprint = Next };

        var switched = waiting.WithServerCertificates(Next, null);

        switched.Should().Be(Saved with { ServerFingerprint = Next });
        switched.Continues(waiting).Should().BeTrue("the same server moved to the certificate it announced");
    }

    [Fact]
    public void WithServerCertificates_forgets_a_cancelled_next_fingerprint()
    {
        (Saved with { NextServerFingerprint = Next }).WithServerCertificates(Current, null).Should().Be(Saved);
    }

    [Fact]
    public void WithServerCertificates_ignores_a_certificate_it_does_not_know()
    {
        Saved.WithServerCertificates(Other, Next).Should().Be(Saved);
    }

    [Fact]
    public void Continues_is_false_for_another_server_or_other_credentials()
    {
        (Saved with { ServerFingerprint = Other }).Continues(Saved).Should().BeFalse("nobody announced that certificate");
        (Saved with { ClientId = "client-other" }).Continues(Saved).Should().BeFalse();
        (Saved with { ServerUrl = "https://elsewhere:5240" }).Continues(Saved).Should().BeFalse();
    }

    [Fact]
    public void ApplyTo_keeps_the_next_fingerprint_and_the_secret_when_the_person_types_it_in()
    {
        var waiting = Saved with { NextServerFingerprint = Next };

        var saved = new ConnectionUpdate("https://lab-rack-3:5240", Next, "client-0a1b2c3d4e", null).ApplyTo(waiting);

        saved.Should().Be(Saved with { ServerFingerprint = Next }, "the server announced that certificate, so the secret carries over and the next one is used up");
    }

    [Fact]
    public void ApplyTo_drops_the_next_fingerprint_for_another_server()
    {
        var waiting = Saved with { NextServerFingerprint = Next };

        var saved = new ConnectionUpdate("https://elsewhere:5240", Other, "client-other", "new-secret").ApplyTo(waiting);

        saved.NextServerFingerprint.Should().BeNull();
    }
}
