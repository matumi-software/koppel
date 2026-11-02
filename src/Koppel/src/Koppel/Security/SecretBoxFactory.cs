using System.Security.Cryptography.X509Certificates;
using Koppel.Core.Security;

namespace Koppel.Security;

internal static partial class SecretBoxFactory
{
    /// <summary>The box that encrypts <c>secrets.json</c> and <c>connection.json</c>.</summary>
    public static SecretBox Create(KoppelPaths paths, ILogger logger)
    {
        var certificate = paths.KeyCertificatePath is { } path
            ? X509CertificateLoader.LoadPkcs12FromFile(path, password: null)
            : null;
        var box = new SecretBox(paths.KeysDirectory, "Koppel.Secrets.v1", certificate);
        if (!box.KeysProtectedAtRest)
        {
            LogKeysUnprotected(logger, paths.KeysDirectory);
        }

        return box;
    }

    [LoggerMessage(Level = LogLevel.Warning, Message = "The key ring in {Directory} is not encrypted at rest. Anyone who can read it can decrypt the secret files. Set Paths:KeyCertificate to a certificate kept outside the data volume (for example a Docker secret).")]
    private static partial void LogKeysUnprotected(ILogger logger, string directory);
}
