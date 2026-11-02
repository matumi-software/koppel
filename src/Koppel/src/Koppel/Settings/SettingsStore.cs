using Koppel.Core.Configuration;

namespace Koppel.Settings;

/// <summary>
/// Owns <c>settings.json</c>, a plain file people may edit by hand. A change is written first; only
/// a write that landed raises <see cref="Changed"/>, which is what the UI waits for.
/// </summary>
/// <remarks><c>Usb:ListenPort</c> in configuration sets the USB port a new install starts with, for hosts where 3240 is taken.</remarks>
internal sealed class SettingsStore(KoppelPaths paths, IConfiguration configuration, ILogger<SettingsStore> logger) : IHostedService, IDisposable
{
    private readonly JsonConfigFile<KoppelSettings> _file = new(
        paths.SettingsFile,
        KoppelSettings.Default with { ListenPort = configuration.GetValue("Usb:ListenPort", KoppelSettings.Default.ListenPort) },
        KoppelJson.Files,
        logger);

    /// <summary>Raised after a change is on disk, with the settings as written.</summary>
    public event Func<KoppelSettings, Task>? Changed
    {
        add => _file.Changed += value;
        remove => _file.Changed -= value;
    }

    public KoppelSettings Current => _file.Current;

    public Task StartAsync(CancellationToken cancellationToken) => _file.StartAsync(cancellationToken);

    public Task StopAsync(CancellationToken cancellationToken)
    {
        _file.Stop();
        return Task.CompletedTask;
    }

    /// <summary>Writes the patched settings to disk, then raises <see cref="Changed"/>.</summary>
    public Task<KoppelSettings> ApplyAsync(SettingsPatch patch, CancellationToken cancellationToken) =>
        _file.UpdateAsync(patch.ApplyTo, cancellationToken);

    public void Dispose() => _file.Dispose();
}
