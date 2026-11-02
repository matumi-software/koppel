using System.Security.Cryptography.X509Certificates;
using Koppel.Core.Security;
using Microsoft.AspNetCore.DataProtection;

namespace Koppel.Security;

internal static partial class SecretBoxFactory
{
    /// <summary>The box that encrypts <c>secrets.json</c> and <c>connection.json</c>.</summary>
    public static SecretBox Create(KoppelPaths paths, ILogger logger)
    {
        var certificate = paths.KeyCertificatePath is { } path
            ? X509CertificateLoader.LoadPkcs12FromFile(path, password: null)
            : null;
        SecretBox box;
        try
        {
            box = new SecretBox(paths.KeysDirectory, "Koppel.Secrets.v1", certificate);
        }
        catch
        {
            certificate?.Dispose();
            throw;
        }

        if (!box.KeysProtectedAtRest)
        {
            LogKeysUnprotected(logger, paths.KeysDirectory);
        }

        return box;
    }

    /// <summary>
    /// ASP.NET Core's own Data Protection (used by the auth stack) keeps its keys next to ours, with the
    /// same protection, instead of in the container's home directory where a restart loses them.
    /// </summary>
    public static void AddAppDataProtection(IServiceCollection services, KoppelPaths paths)
    {
        var directory = Path.Combine(paths.KeysDirectory, "aspnet");
        Directory.CreateDirectory(directory);
        var builder = services.AddDataProtection()
            .SetApplicationName("Koppel")
            .PersistKeysToFileSystem(new DirectoryInfo(directory));
        if (paths.KeyCertificatePath is { } path)
        {
            var certificate = X509CertificateLoader.LoadPkcs12FromFile(path, password: null);
            builder.ProtectKeysWithCertificate(certificate);
            services.AddHostedService(_ => new CertificateLifetime(certificate));
        }
        else if (OperatingSystem.IsWindows())
        {
            builder.ProtectKeysWithDpapi(protectToLocalMachine: true);
        }
    }

    // Data Protection keeps using the certificate for as long as the host runs; this releases it at shutdown.
    private sealed class CertificateLifetime(X509Certificate2 certificate) : IHostedService, IDisposable
    {
        public Task StartAsync(CancellationToken cancellationToken) => Task.CompletedTask;

        public Task StopAsync(CancellationToken cancellationToken) => Task.CompletedTask;

        public void Dispose() => certificate.Dispose();
    }

    [LoggerMessage(Level = LogLevel.Warning, Message = "The key ring in {Directory} is not encrypted at rest. Anyone who can read it can decrypt the secret files. Set Paths:KeyCertificate to a certificate kept outside the data volume (for example a Docker secret).")]
    private static partial void LogKeysUnprotected(ILogger logger, string directory);
}
