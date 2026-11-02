namespace Koppel.Core.Security.Tests;

public sealed class PairingCodeTests
{
    private const string Fingerprint = "EF:8E:16:AF:C3:88:BF:8C:94:CB:98:9A:3F:6A:93:F9:3D:0B:30:60:33:87:79:CF:C0:A4:E6:A4:30:89:14:29";

    [Fact]
    public void Both_ends_derive_the_same_six_digits_however_the_fingerprint_is_written()
    {
        var code = PairingCode.Compute(Fingerprint, "client-nonce", "server-nonce");

        code.Should().MatchRegex("^[0-9]{6}$");
        PairingCode.Compute(Fingerprint.Replace(":", "", StringComparison.Ordinal).ToLowerInvariant(), "client-nonce", "server-nonce").Should().Be(code);
    }

    [Fact]
    public void Another_certificate_or_nonce_gives_another_code()
    {
        var code = PairingCode.Compute(Fingerprint, "a", "b");

        PairingCode.Compute(Fingerprint.Replace("EF", "EE", StringComparison.Ordinal), "a", "b").Should().NotBe(code);
        PairingCode.Compute(Fingerprint, "a2", "b").Should().NotBe(code);
        PairingCode.Compute(Fingerprint, "a", "b2").Should().NotBe(code);
    }

    [Fact]
    public void A_commitment_matches_only_its_own_nonce()
    {
        var nonce = PairingCode.NewNonce();
        var commitment = PairingCode.Commit(nonce);

        PairingCode.Matches(commitment, nonce).Should().BeTrue();
        PairingCode.Matches(commitment.ToLowerInvariant(), nonce).Should().BeTrue();
        PairingCode.Matches(commitment, PairingCode.NewNonce()).Should().BeFalse();
    }

    [Fact]
    public void A_typed_code_may_have_spaces_but_must_be_all_six_digits()
    {
        PairingCode.IsCode("482913", "482 913").Should().BeTrue();
        PairingCode.IsCode("482913", "482914").Should().BeFalse();
        PairingCode.IsCode("482913", "48291").Should().BeFalse();
        PairingCode.IsCode("482913", null).Should().BeFalse();
    }
}
