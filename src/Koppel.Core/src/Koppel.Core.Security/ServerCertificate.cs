using System.Security.Cryptography;
using System.Security.Cryptography.X509Certificates;

namespace Koppel.Core.Security;

/// <summary>
/// The self-signed certificate a Koppel server presents for HTTPS and for the encrypted USB link.
/// Clients trust it by its SHA-256 fingerprint, which the server shows in its UI.
/// </summary>
public static class ServerCertificate
{
    /// <summary>A new self-signed ECDSA P-256 certificate, valid for ten years, exported as PFX.</summary>
    public static byte[] CreatePfx(string hostName)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(hostName);
        using var key = ECDsa.Create(ECCurve.NamedCurves.nistP256);
        var request = new CertificateRequest($"CN={hostName}, O=Koppel", key, HashAlgorithmName.SHA256);
        request.CertificateExtensions.Add(new X509BasicConstraintsExtension(false, false, 0, critical: true));
        request.CertificateExtensions.Add(new X509KeyUsageExtension(X509KeyUsageFlags.DigitalSignature, critical: true));
        request.CertificateExtensions.Add(new X509EnhancedKeyUsageExtension([new Oid("1.3.6.1.5.5.7.3.1")], critical: false));
        var names = new SubjectAlternativeNameBuilder();
        names.AddDnsName(hostName);
        names.AddDnsName("localhost");
        request.CertificateExtensions.Add(names.Build());

        var now = DateTimeOffset.UtcNow;
        using var certificate = request.CreateSelfSigned(now.AddDays(-1), now.AddYears(10));
        return certificate.Export(X509ContentType.Pfx);
    }

    /// <remarks>
    /// Windows' TLS stack cannot use an ephemeral private key for a server, so there the key goes into a
    /// temporary key container that is removed when the certificate is disposed.
    /// </remarks>
    public static X509Certificate2 Load(byte[] pfx) =>
        X509CertificateLoader.LoadPkcs12(pfx, password: null, OperatingSystem.IsWindows() ? X509KeyStorageFlags.DefaultKeySet : X509KeyStorageFlags.EphemeralKeySet);

    /// <summary>Uppercase hex SHA-256 of the certificate, grouped in pairs: <c>3F:9A:…</c>.</summary>
    public static string Fingerprint(X509Certificate2 certificate)
    {
        ArgumentNullException.ThrowIfNull(certificate);
        return string.Join(':', certificate.GetCertHash(HashAlgorithmName.SHA256).Select(b => b.ToString("X2", System.Globalization.CultureInfo.InvariantCulture)));
    }

    /// <summary>
    /// An HTTP handler that accepts exactly the server certificate with this fingerprint and nothing
    /// else. Self-signed is fine; that is the point of pinning.
    /// </summary>
    public static HttpClientHandler CreatePinnedHandler(string pinnedFingerprint) => new()
    {
        ServerCertificateCustomValidationCallback = (_, certificate, _, _) => Matches(certificate, pinnedFingerprint),
    };

    /// <summary>
    /// An HTTP handler that accepts a server certificate only when <paramref name="accept"/> says so, for a
    /// pin that can hold more than one fingerprint, such as the server's next certificate during a handover.
    /// </summary>
    public static HttpClientHandler CreatePinnedHandler(Func<X509Certificate2?, bool> accept)
    {
        ArgumentNullException.ThrowIfNull(accept);
        return new() { ServerCertificateCustomValidationCallback = (_, certificate, _, _) => accept(certificate) };
    }

    /// <summary>True when <paramref name="certificate"/> has any of the pinned fingerprints. Null and empty ones match nothing.</summary>
    public static bool MatchesAny(X509Certificate2? certificate, params IEnumerable<string?> pinnedFingerprints)
    {
        ArgumentNullException.ThrowIfNull(pinnedFingerprints);
        return pinnedFingerprints.Any(pinned => Matches(certificate, pinned));
    }

    /// <summary>True when <paramref name="certificate"/> has the pinned fingerprint. Spaces, colons and case are ignored.</summary>
    public static bool Matches(X509Certificate2? certificate, string? pinnedFingerprint)
    {
        if (certificate is null || string.IsNullOrWhiteSpace(pinnedFingerprint))
        {
            return false;
        }

        static string Normalize(string value) => new string(value.Where(Uri.IsHexDigit).ToArray()).ToUpperInvariant();
        return CryptographicOperations.FixedTimeEquals(
            System.Text.Encoding.ASCII.GetBytes(Normalize(Fingerprint(certificate))),
            System.Text.Encoding.ASCII.GetBytes(Normalize(pinnedFingerprint)));
    }
}
