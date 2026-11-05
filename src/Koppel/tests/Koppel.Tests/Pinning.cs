using System.Security.Cryptography.X509Certificates;
using Skakel.Protocol;

namespace Koppel.Tests;

/// <summary>Handlers that trust exactly one server certificate, as a Koppel client does.</summary>
internal static class Pinning
{
    public static HttpClientHandler Handler(string fingerprint) =>
        new() { ServerCertificateCustomValidationCallback = (_, certificate, _, _) => Fingerprints.Matches(certificate, fingerprint) };

    public static bool Matches(X509Certificate? certificate, string fingerprint) =>
        Fingerprints.Matches(certificate as X509Certificate2, fingerprint);
}
