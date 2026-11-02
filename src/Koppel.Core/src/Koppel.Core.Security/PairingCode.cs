using System.Buffers.Binary;
using System.Globalization;
using System.Security.Cryptography;
using System.Text;

namespace Koppel.Core.Security;

/// <summary>
/// The six-digit code a person types on the server to approve a client, as with Moonlight and
/// Sunshine. The protocol behind it is numeric comparison with a commitment, as in Bluetooth pairing:
/// <list type="number">
/// <item>The client sends <see cref="Commit"/> of a random nonce, not the nonce.</item>
/// <item>The server answers with its own random nonce.</item>
/// <item>The client reveals its nonce; the server checks it against the commitment.</item>
/// </list>
/// Both ends then derive the code from the certificate the client pinned and both nonces. A man in
/// the middle has its own certificate and must fix each nonce before it sees the other side's, so it
/// cannot steer the two codes to match: it gets one guess in a million. The code is never sent.
/// </summary>
public static class PairingCode
{
    public const int Digits = 6;

    /// <summary>A fresh random nonce, as base64url.</summary>
    public static string NewNonce() => Base64Url(RandomNumberGenerator.GetBytes(32));

    /// <summary>The commitment a client sends before revealing its nonce.</summary>
    public static string Commit(string nonce) => Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes(nonce)));

    public static bool Matches(string commitment, string nonce) =>
        CryptographicOperations.FixedTimeEquals(Encoding.ASCII.GetBytes(commitment.ToUpperInvariant()), Encoding.ASCII.GetBytes(Commit(nonce)));

    /// <summary>The code, such as <c>482913</c>. Fingerprints compare by their hex digits alone.</summary>
    public static string Compute(string fingerprint, string clientNonce, string serverNonce)
    {
        ArgumentNullException.ThrowIfNull(fingerprint);
        var hex = new string(fingerprint.Where(Uri.IsHexDigit).ToArray()).ToUpperInvariant();
        var hash = SHA256.HashData(Encoding.UTF8.GetBytes($"koppel-pair-v1|{hex}|{clientNonce}|{serverNonce}"));
        var value = BinaryPrimitives.ReadUInt32BigEndian(hash) % 1_000_000;
        return value.ToString("D6", CultureInfo.InvariantCulture);
    }

    /// <summary>Compares a typed code, ignoring spaces, in constant time.</summary>
    public static bool IsCode(string expected, string? typed)
    {
        ArgumentNullException.ThrowIfNull(expected);
        var digits = new string((typed ?? "").Where(char.IsAsciiDigit).ToArray());
        return digits.Length == Digits
            && CryptographicOperations.FixedTimeEquals(Encoding.ASCII.GetBytes(expected), Encoding.ASCII.GetBytes(digits));
    }

    private static string Base64Url(byte[] bytes) =>
        Convert.ToBase64String(bytes).TrimEnd('=').Replace('+', '-').Replace('/', '_');
}
