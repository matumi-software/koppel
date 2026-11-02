using System.Globalization;
using System.Security.Cryptography;
using System.Text;

namespace Koppel.Core.Security;

/// <summary>Creates, hashes and checks client secrets.</summary>
public static class Secrets
{
    /// <summary>A new random secret, 32 bytes as base64url (43 characters).</summary>
    public static string NewSecret() => Base64Url(RandomNumberGenerator.GetBytes(32));

    /// <summary>A new random identifier with a readable prefix, such as <c>client-3f9a1c2b7d</c>.</summary>
    public static string NewId(string prefix) =>
        $"{prefix}-{Convert.ToHexStringLower(RandomNumberGenerator.GetBytes(5))}";

    private const string Sha256Prefix = "sha256$";
    private const string Pbkdf2Prefix = "pbkdf2-sha256";

    /// <summary>
    /// SHA-256 hash, stored as <c>sha256$hash</c>. A client secret is 32 random bytes, so there is
    /// nothing to guess and a slow, salted hash would only make every token request expensive.
    /// </summary>
    public static string Hash(string secret)
    {
        ArgumentException.ThrowIfNullOrEmpty(secret);
        return Sha256Prefix + Convert.ToBase64String(SHA256.HashData(Encoding.UTF8.GetBytes(secret)));
    }

    /// <summary>True for a hash in an older format, which still verifies but should be replaced by <see cref="Hash"/>.</summary>
    public static bool IsLegacy(string storedHash) => storedHash?.StartsWith(Pbkdf2Prefix + "$", StringComparison.Ordinal) == true;

    /// <summary>Checks a secret against a stored hash in constant time. Reads the current format and the older PBKDF2 one.</summary>
    public static bool Verify(string secret, string storedHash)
    {
        if (string.IsNullOrEmpty(secret) || string.IsNullOrEmpty(storedHash))
        {
            return false;
        }

        try
        {
            if (storedHash.StartsWith(Sha256Prefix, StringComparison.Ordinal))
            {
                var expectedHash = Convert.FromBase64String(storedHash[Sha256Prefix.Length..]);
                return CryptographicOperations.FixedTimeEquals(SHA256.HashData(Encoding.UTF8.GetBytes(secret)), expectedHash);
            }

            var parts = storedHash.Split('$');
            if (parts.Length != 4 || parts[0] != Pbkdf2Prefix || !int.TryParse(parts[1], NumberStyles.None, CultureInfo.InvariantCulture, out var iterations) || iterations is < 1 or > 10_000_000)
            {
                return false;
            }

            var salt = Convert.FromBase64String(parts[2]);
            var expected = Convert.FromBase64String(parts[3]);
            var actual = Rfc2898DeriveBytes.Pbkdf2(Encoding.UTF8.GetBytes(secret), salt, iterations, HashAlgorithmName.SHA256, expected.Length);
            return CryptographicOperations.FixedTimeEquals(actual, expected);
        }
        catch (FormatException)
        {
            return false;
        }
    }

    private static string Base64Url(byte[] bytes) =>
        Convert.ToBase64String(bytes).TrimEnd('=').Replace('+', '-').Replace('/', '_');
}
