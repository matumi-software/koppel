using Koppel.Client;
using Koppel.Login;
using Koppel.Settings;
using Koppel.Usb;

namespace Koppel.Tray;

/// <summary>
/// What the service tells its tray icons. <see cref="Current"/> reads the stores each time it is asked; <see cref="WaitAsync"/>
/// wakes the launcher as soon as one of them changes, so the tooltip follows within a moment rather than at the next poll.
/// The setup window has no event of its own; the launcher's poll catches it closing.
/// </summary>
internal sealed class TrayFeed(SettingsStore settings, UiAuthStore auth, UsbClient client, UsbServer server, ConnectionStore connection)
    : IHostedService
{
    private TaskCompletionSource _changed = NewSignal();

    public TrayState Current => TrayStatus.Describe(new TrayFacts(
        settings.Current.Role,
        auth.HasPassword,
        auth.SetupOpen,
        client.Current,
        HostOf(connection.View.ServerUrl),
        server.Current));

    /// <summary>Completes when something the state is made from has changed since the last call.</summary>
    public Task WaitAsync() => Volatile.Read(ref _changed).Task;

    public Task StartAsync(CancellationToken cancellationToken)
    {
        settings.Changed += OnSettings;
        client.Changed += OnClient;
        server.Changed += OnServer;
        connection.Changed += OnConnection;
        auth.StampChanged += OnStamp;
        return Task.CompletedTask;
    }

    public Task StopAsync(CancellationToken cancellationToken)
    {
        settings.Changed -= OnSettings;
        client.Changed -= OnClient;
        server.Changed -= OnServer;
        connection.Changed -= OnConnection;
        auth.StampChanged -= OnStamp;
        Signal();
        return Task.CompletedTask;
    }

    private Task OnSettings(KoppelSettings value) => Signal();

    private Task OnClient(ClientDevices value) => Signal();

    private Task OnServer(ServerDevices value) => Signal();

    private Task OnConnection(ConnectionView value) => Signal();

    private Task OnStamp(UiStampChange value) => Signal();

    private Task Signal()
    {
        Interlocked.Exchange(ref _changed, NewSignal()).TrySetResult();
        return Task.CompletedTask;
    }

    private static TaskCompletionSource NewSignal() => new(TaskCreationOptions.RunContinuationsAsynchronously);

    /// <summary>The host of the server's address, as the UI's sidebar names it.</summary>
    private static string? HostOf(string? url) => Uri.TryCreate(url, UriKind.Absolute, out var uri) ? uri.Host : null;
}
