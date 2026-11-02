using System.Globalization;
using System.Security.Cryptography;
using System.Text;

namespace Koppel.Login;

/// <summary>The UI password: what is allowed, and how it is hashed for <c>ui-auth.json</c>.</summary>
internal static class UiPassword
{
    public const int MinLength = 10;
    public const int MaxLength = 128;

    /// <summary>OWASP's figure for PBKDF2-HMAC-SHA256. A stored hash with fewer is not accepted.</summary>
    public const int Iterations = 600_000;

    private const string Prefix = "pbkdf2-sha256";
    private const int SaltBytes = 16;
    private const int HashBytes = 32;

    private static readonly string[] Obvious = ["koppel", "password"];

    /// <summary>Why a password is not acceptable, worded for the person choosing it, or null.</summary>
    public static string? Validate(string? password)
    {
        if (string.IsNullOrEmpty(password) || password.Length < MinLength)
        {
            return $"Use at least {MinLength} characters.";
        }

        if (password.Length > MaxLength)
        {
            return $"Use at most {MaxLength} characters.";
        }

        return Obvious.Any(word => string.Equals(password, word, StringComparison.OrdinalIgnoreCase))
            ? "That password is too easy to guess."
            : null;
    }

    /// <summary>Stored as <c>pbkdf2-sha256$iterations$salt$hash</c>, so the parameters travel with the hash.</summary>
    public static string Hash(string password)
    {
        var salt = RandomNumberGenerator.GetBytes(SaltBytes);
        var hash = Rfc2898DeriveBytes.Pbkdf2(Encoding.UTF8.GetBytes(password), salt, Iterations, HashAlgorithmName.SHA256, HashBytes);
        return string.Create(CultureInfo.InvariantCulture, $"{Prefix}${Iterations}${Convert.ToBase64String(salt)}${Convert.ToBase64String(hash)}");
    }

    public static bool Verify(string password, string stored)
    {
        if (!TryParse(stored, out var iterations, out var salt, out var expected))
        {
            return false;
        }

        var actual = Rfc2898DeriveBytes.Pbkdf2(Encoding.UTF8.GetBytes(password), salt, iterations, HashAlgorithmName.SHA256, expected.Length);
        return CryptographicOperations.FixedTimeEquals(actual, expected);
    }

    /// <summary>True for a hash this code wrote: right format and at least the iteration count above.</summary>
    public static bool IsWellFormed(string stored) => TryParse(stored, out _, out _, out _);

    private static bool TryParse(string stored, out int iterations, out byte[] salt, out byte[] hash)
    {
        iterations = 0;
        salt = [];
        hash = [];
        var parts = stored.Split('$');
        if (parts.Length != 4 || parts[0] != Prefix || !int.TryParse(parts[1], NumberStyles.None, CultureInfo.InvariantCulture, out iterations) || iterations < Iterations)
        {
            return false;
        }

        try
        {
            salt = Convert.FromBase64String(parts[2]);
            hash = Convert.FromBase64String(parts[3]);
        }
        catch (FormatException)
        {
            return false;
        }

        return salt.Length >= SaltBytes && hash.Length == HashBytes;
    }
}
