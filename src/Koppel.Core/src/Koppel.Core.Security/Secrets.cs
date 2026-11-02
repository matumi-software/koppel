using System.Globalization;
using System.Security.Cryptography;
using System.Text;

namespace Koppel.Core.Security;

/// <summary>Creates, hashes and checks client secrets.</summary>
public static class Secrets
{
    private const int Iterations = 210_000;
    private const int SaltBytes = 16;
    private const int HashBytes = 32;

    /// <summary>A new random secret, 32 bytes as base64url (43 characters).</summary>
    public static string NewSecret() => Base64Url(RandomNumberGenerator.GetBytes(32));

    /// <summary>A new random identifier with a readable prefix, such as <c>client-3f9a1c2b7d</c>.</summary>
    public static string NewId(string prefix) =>
        $"{prefix}-{Convert.ToHexStringLower(RandomNumberGenerator.GetBytes(5))}";

    /// <summary>PBKDF2-SHA256 hash, stored as <c>pbkdf2-sha256$iterations$salt$hash</c>.</summary>
    public static string Hash(string secret)
    {
        ArgumentException.ThrowIfNullOrEmpty(secret);
        var salt = RandomNumberGenerator.GetBytes(SaltBytes);
        var hash = Rfc2898DeriveBytes.Pbkdf2(Encoding.UTF8.GetBytes(secret), salt, Iterations, HashAlgorithmName.SHA256, HashBytes);
        return string.Create(CultureInfo.InvariantCulture, $"pbkdf2-sha256${Iterations}${Convert.ToBase64String(salt)}${Convert.ToBase64String(hash)}");
    }

    /// <summary>Checks a secret against a stored hash in constant time.</summary>
    public static bool Verify(string secret, string storedHash)
    {
        if (string.IsNullOrEmpty(secret) || string.IsNullOrEmpty(storedHash))
        {
            return false;
        }

        var parts = storedHash.Split('$');
        if (parts.Length != 4 || parts[0] != "pbkdf2-sha256" || !int.TryParse(parts[1], NumberStyles.None, CultureInfo.InvariantCulture, out var iterations))
        {
            return false;
        }

        try
        {
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
