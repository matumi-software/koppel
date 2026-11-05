using Koppel.Settings;
using Skakel.Server;

namespace Koppel.Usb;

/// <summary>
/// Ends the remote clients' hub connections and streams when the role leaves the server side. The role is
/// only checked when a request starts, so what is already open would otherwise carry on. Skakel ends them
/// itself when a client is revoked or rotated, and when Koppel stops.
/// </summary>
internal sealed partial class LiveSessions(SkakelSessions sessions, SettingsStore settings, ILogger<LiveSessions> logger) : IHostedService
{
    private int _served;

    public Task StartAsync(CancellationToken cancellationToken)
    {
        _served = Serves(settings.Current.Role) ? 1 : 0;
        settings.Changed += OnSettingsChanged;
        return Task.CompletedTask;
    }

    public Task StopAsync(CancellationToken cancellationToken)
    {
        settings.Changed -= OnSettingsChanged;
        return Task.CompletedTask;
    }

    private static bool Serves(KoppelRole role) => role is KoppelRole.Server or KoppelRole.Both;

    private async Task OnSettingsChanged(KoppelSettings next)
    {
        var serves = Serves(next.Role);
        if (Interlocked.Exchange(ref _served, serves ? 1 : 0) == 1 && !serves)
        {
            LogEnding();
            await sessions.EndAllAsync();
        }
    }

    [LoggerMessage(Level = LogLevel.Information, Message = "This machine is no longer a server; ending the remote clients' connections")]
    private partial void LogEnding();
}
