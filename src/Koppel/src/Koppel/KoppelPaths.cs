using Koppel.Core.Security;

namespace Koppel;

/// <summary>Where Koppel keeps its files on disk.</summary>
/// <param name="ConfigDirectory">Config files: the plain <c>settings.json</c>, <c>devices.json</c> and <c>attached.json</c>, and the encrypted <c>secrets.json</c>, <c>connection.json</c> and <c>ui-auth.json</c>.</param>
/// <param name="KeysDirectory">The key ring that encrypts the secret files. Kept apart from them.</param>
/// <param name="KeyCertificatePath">Optional PFX that protects the key ring itself, such as a Docker secret.</param>
internal sealed record KoppelPaths(string ConfigDirectory, string KeysDirectory, string? KeyCertificatePath = null)
{
    public string SettingsFile => Path.Combine(ConfigDirectory, "settings.json");

    /// <summary>Server role: signing key, HTTPS certificate and registered clients. Encrypted.</summary>
    public string ServerSecretsFile => Path.Combine(ConfigDirectory, "secrets.json");

    /// <summary>Client role: server address, pinned fingerprint, client ID and secret. Encrypted.</summary>
    public string ConnectionFile => Path.Combine(ConfigDirectory, "connection.json");

    /// <summary>The UI's password hash and security stamp. Encrypted.</summary>
    public string UiAuthFile => Path.Combine(ConfigDirectory, "ui-auth.json");

    /// <summary>
    /// The one-time setup code, while no UI password is set. It sits in the config directory, which
    /// <see cref="Prepare"/> restricts to administrators.
    /// </summary>
    public string SetupCodeFile => Path.Combine(ConfigDirectory, "setup-code.txt");

    /// <summary>
    /// Creates the config and keys directories and restricts both to administrators, before anything is written to them.
    /// On Windows the data root above them (<c>%ProgramData%\Koppel</c>, which also holds the installer's driver record) is
    /// restricted too, so a standard user cannot plant files in it or swap what is under it.
    /// </summary>
    public void Prepare()
    {
        var root = Path.GetDirectoryName(ConfigDirectory);
        var dataRoot = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.CommonApplicationData), "Koppel");
        var directories = OperatingSystem.IsWindows() && string.Equals(root, dataRoot, StringComparison.OrdinalIgnoreCase)
            ? new[] { root!, ConfigDirectory, KeysDirectory }
            : [ConfigDirectory, KeysDirectory];
        foreach (var directory in directories)
        {
            Directory.CreateDirectory(directory);
            DirectoryHardening.Harden(directory);
        }
    }

    /// <summary>
    /// Reads <c>Paths:Config</c>, <c>Paths:Keys</c> and <c>Paths:KeyCertificate</c>, resolving relative
    /// paths against the content root. Defaults: <c>%ProgramData%\Koppel\{config,keys}</c> on Windows,
    /// <c>/var/lib/koppel/{config,keys}</c> elsewhere.
    /// </summary>
    public static KoppelPaths FromConfiguration(IConfiguration configuration, IHostEnvironment environment)
    {
        string Resolve(string key, string fallback) =>
            configuration[key] is { Length: > 0 } configured
                ? Path.GetFullPath(configured, environment.ContentRootPath)
                : fallback;

        var root = OperatingSystem.IsWindows()
            ? Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.CommonApplicationData), "Koppel")
            : "/var/lib/koppel";
        var certificate = configuration["Paths:KeyCertificate"] is { Length: > 0 } path
            ? Path.GetFullPath(path, environment.ContentRootPath)
            : null;

        return new KoppelPaths(
            Resolve("Paths:Config", Path.Combine(root, "config")),
            Resolve("Paths:Keys", Path.Combine(root, "keys")),
            certificate);
    }
}
