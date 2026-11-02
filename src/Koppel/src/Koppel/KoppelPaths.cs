namespace Koppel;

/// <summary>Where Koppel keeps its files on disk.</summary>
internal sealed record KoppelPaths(string ConfigDirectory)
{
    public string SettingsFile => Path.Combine(ConfigDirectory, "settings.json");

    /// <summary>
    /// Reads <c>Paths:Config</c>, resolved against the content root when relative. Without it the
    /// default is <c>%ProgramData%\Koppel\config</c> on Windows and <c>/var/lib/koppel/config</c>
    /// elsewhere. Docker sets <c>Paths__Config</c> to a path inside the volume.
    /// </summary>
    public static KoppelPaths FromConfiguration(IConfiguration configuration, IHostEnvironment environment)
    {
        var configured = configuration["Paths:Config"];
        var directory = string.IsNullOrWhiteSpace(configured)
            ? DefaultConfigDirectory()
            : Path.GetFullPath(configured, environment.ContentRootPath);
        return new KoppelPaths(directory);
    }

    private static string DefaultConfigDirectory() =>
        OperatingSystem.IsWindows()
            ? Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.CommonApplicationData), "Koppel", "config")
            : "/var/lib/koppel/config";
}
