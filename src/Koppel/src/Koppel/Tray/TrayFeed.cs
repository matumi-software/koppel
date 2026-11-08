using Koppel.Client;
using Koppel.Core.Activity;
using Koppel.Login;
using Koppel.Settings;
using Koppel.Usb;
using Skakel.Server;

namespace Koppel.Tray;

/// <summary>
/// What the service tells its tray icons. <see cref="Current"/> reads the stores each time it is asked; <see cref="WaitAsync"/>
/// wakes the launcher as soon as one of them changes, so the tooltip follows within a moment rather than at the next poll.
/// The setup window has no event of its own; the launcher's poll catches it closing. Notifications (<see cref="TrayNoticeRules"/>)
/// queue up here until the launcher takes them.
/// </summary>
internal sealed class TrayFeed(
    SettingsStore settings,
    UiAuthStore auth,
    UsbClient client,
    UsbServer server,
    ConnectionStore connection,
    InMemoryActivityStore activity,
    SkakelPairing pairing) : IHostedService
{
    private readonly Lock _lock = new();
    private readonly TrayNoticeRules _rules = new();
    private readonly List<TrayNotice> _notices = [];
    private TaskCompletionSource _changed = NewSignal();

    public TrayState Current => TrayStatus.Describe(new TrayFacts(
        settings.Current.Role,
        auth.HasPassword,
        auth.SetupOpen,
        client.Current,
        HostOf(connection.View.ServerUrl),
        server.Current));

    /// <summary>Completes when something the state is made from has changed, or a notice came, since the last call.</summary>
    public Task WaitAsync() => Volatile.Read(ref _changed).Task;

    /// <summary>The notices queued since the last call, oldest first.</summary>
    public IReadOnlyList<TrayNotice> TakeNotices()
    {
        lock (_lock)
        {
            var taken = _notices.ToList();
            _notices.Clear();
            return taken;
        }
    }

    public Task StartAsync(CancellationToken cancellationToken)
    {
        settings.Changed += OnSettings;
        client.Changed += OnClient;
        server.Changed += OnServer;
        connection.Changed += OnConnection;
        auth.StampChanged += OnStamp;
        activity.Recorded += OnActivity;
        pairing.Changed += OnPairing;
        lock (_lock)
        {
            // The requests already waiting when Koppel starts were not told; a request is told when it arrives.
            _rules.FromPairing(Requests(pairing.Overview), KoppelSettings.Default with { NotifyPairing = false });
        }

        return Task.CompletedTask;
    }

    public Task StopAsync(CancellationToken cancellationToken)
    {
        settings.Changed -= OnSettings;
        client.Changed -= OnClient;
        server.Changed -= OnServer;
        connection.Changed -= OnConnection;
        auth.StampChanged -= OnStamp;
        activity.Recorded -= OnActivity;
        pairing.Changed -= OnPairing;
        Signal();
        return Task.CompletedTask;
    }

    private Task OnSettings(KoppelSettings value) => Signal();

    private Task OnClient(ClientDevices value) => Signal();

    private Task OnServer(ServerDevices value) => Signal();

    private Task OnConnection(ConnectionView value) => Signal();

    private Task OnStamp(UiStampChange value) => Signal();

    private Task OnActivity(ActivityEvent value)
    {
        lock (_lock)
        {
            if (_rules.FromActivity(value, settings.Current) is not { } notice)
            {
                return Task.CompletedTask;
            }

            _notices.Add(notice);
        }

        return Signal();
    }

    private Task OnPairing(SkakelPairingOverview value)
    {
        lock (_lock)
        {
            var notices = _rules.FromPairing(Requests(value), settings.Current);
            if (notices.Count == 0)
            {
                return Task.CompletedTask;
            }

            _notices.AddRange(notices);
        }

        return Signal();
    }

    private static List<TrayPairingRequest> Requests(SkakelPairingOverview overview) =>
        [.. overview.Requests.Select(request => new TrayPairingRequest(request.Id, request.ClientName, request.State))];

    private Task Signal()
    {
        Interlocked.Exchange(ref _changed, NewSignal()).TrySetResult();
        return Task.CompletedTask;
    }

    private static TaskCompletionSource NewSignal() => new(TaskCreationOptions.RunContinuationsAsynchronously);

    /// <summary>The host of the server's address, as the UI's sidebar names it.</summary>
    private static string? HostOf(string? url) => Uri.TryCreate(url, UriKind.Absolute, out var uri) ? uri.Host : null;
}
