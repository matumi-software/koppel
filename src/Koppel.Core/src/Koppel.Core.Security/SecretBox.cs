using System.Security.Cryptography.X509Certificates;
using Koppel.Core.Configuration;
using Microsoft.AspNetCore.DataProtection;

namespace Koppel.Core.Security;

/// <summary>
/// Encrypts and decrypts with .NET Data Protection over a key ring kept in its own directory.
/// </summary>
/// <remarks>
/// The key ring must not sit next to what it protects, or the encryption only hides the data.
/// How the key ring itself is protected at rest:
/// <list type="bullet">
/// <item>Windows: machine-scoped DPAPI, so a service account change does not lose the keys.</item>
/// <item>With a key certificate (any OS): the keys are encrypted to that certificate. Mount it from a
/// Docker secret so it never lives in the data volume.</item>
/// <item>Otherwise (Linux without a certificate): the directory is <c>chmod 700</c> and the keys are
/// plain on disk. Callers should warn about this.</item>
/// </list>
/// </remarks>
public sealed class SecretBox : IDisposable
{
    private readonly IDataProtectionProvider _provider;
    private readonly IDataProtector _protector;
    private readonly X509Certificate2? _keyCertificate;

    public SecretBox(string keyRingDirectory, string purpose, X509Certificate2? keyCertificate = null)
    {
        Directory.CreateDirectory(keyRingDirectory);
        DirectoryHardening.Harden(keyRingDirectory);

        _keyCertificate = keyCertificate;
        KeysProtectedAtRest = keyCertificate is not null || OperatingSystem.IsWindows();
        _provider = DataProtectionProvider.Create(new DirectoryInfo(keyRingDirectory), builder =>
        {
            builder.SetApplicationName("Koppel");
            if (keyCertificate is not null)
            {
                builder.ProtectKeysWithCertificate(keyCertificate);
            }
            else if (OperatingSystem.IsWindows())
            {
                builder.ProtectKeysWithDpapi(protectToLocalMachine: true);
            }
        });
        _protector = _provider.CreateProtector(purpose);
    }

    /// <summary>False when the key ring itself is plain on disk (Linux without a key certificate).</summary>
    public bool KeysProtectedAtRest { get; }

    public byte[] Protect(byte[] plaintext) => _protector.Protect(plaintext);

    public byte[] Unprotect(byte[] protectedData) => _protector.Unprotect(protectedData);

    /// <summary>A codec that encrypts a <see cref="JsonConfigFile{T}"/> with this box.</summary>
    public IFileCodec AsFileCodec() => new ProtectedFileCodec(this);

    /// <summary>Also disposes the key certificate: the box takes ownership of it.</summary>
    public void Dispose()
    {
        (_provider as IDisposable)?.Dispose();
        _keyCertificate?.Dispose();
    }

    private sealed class ProtectedFileCodec(SecretBox box) : IFileCodec
    {
        public byte[] Encode(string json) => box.Protect(PlainTextCodec.Instance.Encode(json));

        public string Decode(byte[] bytes) => PlainTextCodec.Instance.Decode(box.Unprotect(bytes));
    }
}
