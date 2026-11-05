using System.Net;
using System.Net.Sockets;
using Koppel.Core.Configuration;
using Koppel.Settings;
using Skakel.Client;
using Skakel.Protocol;
using Skakel.Server;

namespace Koppel.Discovery;

/// <summary>A server found on the local network, as the UI shows it.</summary>
/// <param name="Address">Where its answer came from, which is where to reach it.</param>
/// <param name="Version">Koppel's version on that server.</param>
/// <param name="OtherAddresses">
/// Other addresses that answered with the same fingerprint, with the port when it differs. A machine on
/// more than one network does that, and so does one that copies another server's fingerprint.
/// </param>
public sealed record DiscoveredServer(string Name, string Address, int HttpsPort, string Fingerprint, string? Version, IReadOnlyList<string> OtherAddresses)
{
    public string Url => IPAddress.TryParse(Address, out var ip) && ip.AddressFamily == AddressFamily.InterNetworkV6
        ? $"https://[{Address}]:{HttpsPort}"
        : $"https://{Address}:{HttpsPort}";

    public static DiscoveredServer From(SkakelDiscoveredServer server) =>
        new(server.Name, server.Address.ToString(), server.Port, server.Fingerprint, server.App.Version,
            [.. server.OtherEndpoints.Select(e => e.Port == server.Port ? e.Address.ToString() : e.ToString())]);
}

/// <summary>
/// Answers Skakel discovery probes for this server on UDP, in the protocol's format (see Skakel's
/// <c>docs/protocol.md</c>). Koppel answers them itself rather than through Skakel's discovery service,
/// because it answers only while its role is server or both, and the role changes while it runs.
/// </summary>
internal sealed partial class ServerAnnouncer(ILogger<ServerAnnouncer> logger) : IDisposable
{
    private readonly Lock _lock = new();
    private Socket? _socket;
    private CancellationTokenSource? _stop;

    public bool IsAnnouncing
    {
        get
        {
            lock (_lock)
            {
                return _socket is not null;
            }
        }
    }

    /// <summary>Starts answering on UDP <paramref name="udpPort"/> with <paramref name="answer"/>, or replaces what the answers say.</summary>
    public void Announce(DiscoveryAnswer answer, int udpPort)
    {
        ArgumentNullException.ThrowIfNull(answer);
        var reply = SkakelDiscovery.CreateAnswer(answer);
        lock (_lock)
        {
            StopLocked();
            var socket = new Socket(AddressFamily.InterNetwork, SocketType.Dgram, ProtocolType.Udp);
            try
            {
                socket.Bind(new IPEndPoint(IPAddress.Any, udpPort));
            }
            catch
            {
                socket.Dispose();
                throw;
            }

            _socket = socket;
            _stop = new CancellationTokenSource();
            _ = AnswerAsync(socket, reply, _stop.Token);
            LogAnnouncing(answer.Name, udpPort);
        }
    }

    public void Stop()
    {
        lock (_lock)
        {
            StopLocked();
        }
    }

    public void Dispose() => Stop();

    private void StopLocked()
    {
        if (_socket is null)
        {
            return;
        }

        _stop?.Cancel();
        _stop?.Dispose();
        _socket.Dispose();
        _stop = null;
        _socket = null;
        LogStopped();
    }

    /// <summary>Answers a full-size probe only, and never with more bytes than it had, so nobody can use this server to multiply traffic.</summary>
    private async Task AnswerAsync(Socket socket, byte[] reply, CancellationToken cancellationToken)
    {
        var buffer = new byte[2048];
        var anyone = new IPEndPoint(IPAddress.Any, 0);
        while (!cancellationToken.IsCancellationRequested)
        {
            try
            {
                var received = await socket.ReceiveFromAsync(buffer, SocketFlags.None, anyone, cancellationToken);
                if (SkakelDiscovery.IsProbe(buffer.AsSpan(0, received.ReceivedBytes)) && reply.Length <= received.ReceivedBytes)
                {
                    await socket.SendToAsync(reply, SocketFlags.None, received.RemoteEndPoint, cancellationToken);
                }
            }
            catch (Exception ex) when (ex is OperationCanceledException or ObjectDisposedException)
            {
                return;
            }
            catch (SocketException ex) when (ex.SocketErrorCode is SocketError.ConnectionReset or SocketError.NetworkReset or SocketError.MessageSize)
            {
                // Windows reports these on the next receive, each about one datagram: an earlier answer that could
                // not be delivered or ran out of hops, or a datagram larger than the buffer, which it drops. Anyone
                // on the network can send the last one, so none of them may stop the answers.
            }
            catch (SocketException ex)
            {
                LogAnswerFailed(ex);
                return;
            }
        }
    }

    [LoggerMessage(Level = LogLevel.Information, Message = "Answering discovery on the local network as {Name} (UDP port {Port})")]
    private partial void LogAnnouncing(string name, int port);

    [LoggerMessage(Level = LogLevel.Information, Message = "Stopped answering discovery on the local network")]
    private partial void LogStopped();

    [LoggerMessage(Level = LogLevel.Warning, Message = "Stopped answering discovery after a network error")]
    private partial void LogAnswerFailed(Exception exception);
}

/// <summary>
/// Server role: announces this server on the local network so clients can find it, and stops when
/// the machine becomes a client only. A switch to a new certificate is announced at once.
/// <c>Discovery:Enabled=false</c> turns it off.
/// </summary>
internal sealed partial class ServerAnnouncerService(
    ServerAnnouncer announcer,
    SettingsStore settings,
    SkakelKeys keys,
    HostInfo host,
    IConfiguration configuration,
    ILogger<ServerAnnouncerService> logger) : IHostedService
{
    private readonly Lock _gate = new();
    private bool _enabled;
    private string? _announced;

    public Task StartAsync(CancellationToken cancellationToken)
    {
        _enabled = configuration.GetValue("Discovery:Enabled", true);
        if (_enabled)
        {
            settings.Changed += ApplyAsync;
            keys.Changed += ApplyKeysAsync;
            Apply(settings.Current);
        }

        return Task.CompletedTask;
    }

    public Task StopAsync(CancellationToken cancellationToken)
    {
        settings.Changed -= ApplyAsync;
        keys.Changed -= ApplyKeysAsync;
        announcer.Stop();
        return Task.CompletedTask;
    }

    private Task ApplyAsync(KoppelSettings value)
    {
        Apply(value);
        return Task.CompletedTask;
    }

    private Task ApplyKeysAsync(SkakelKeysView value)
    {
        Apply(settings.Current);
        return Task.CompletedTask;
    }

    /// <summary>Under one lock, so a settings change and a certificate switch at once cannot leave the old fingerprint announced.</summary>
    private void Apply(KoppelSettings value)
    {
        lock (_gate)
        {
            ApplyLocked(value);
        }
    }

    private void ApplyLocked(KoppelSettings value)
    {
        var shouldAnnounce = value.Role != KoppelRole.Client;
        var fingerprint = keys.Fingerprint;
        if (shouldAnnounce == announcer.IsAnnouncing && (!shouldAnnounce || fingerprint == _announced))
        {
            return;
        }

        try
        {
            if (shouldAnnounce)
            {
                var app = new SkakelApp(SystemEndpoints.AppId, SystemEndpoints.Version);
                announcer.Announce(new DiscoveryAnswer(Environment.MachineName, host.NetworkPort, fingerprint, app, SkakelProtocol.Versions), host.DiscoveryPort);
                _announced = fingerprint;
            }
            else
            {
                announcer.Stop();
            }
        }
        catch (SocketException ex)
        {
            // Another program holds the UDP port. Clients can still enter this server's address.
            LogAnnounceFailed(ex);
        }
    }

    [LoggerMessage(Level = LogLevel.Warning, Message = "Could not announce on the local network; clients must enter this server's address")]
    private partial void LogAnnounceFailed(Exception exception);
}

/// <summary>
/// Client role: finds Koppel servers on the local network with Skakel's browser while this machine can be
/// a client, leaving out itself. The same browser lets the link follow its server to another address.
/// <c>Discovery:Enabled=false</c> turns it off.
/// </summary>
internal sealed partial class ServerFinder(
    SkakelBrowser browser,
    SettingsStore settings,
    SkakelKeys keys,
    IConfiguration configuration,
    ILogger<ServerFinder> logger) : IHostedService
{
    public event Func<IReadOnlyList<DiscoveredServer>, Task>? Changed;

    public IReadOnlyList<DiscoveredServer> Servers => Others(browser.Servers);

    public Task StartAsync(CancellationToken cancellationToken)
    {
        if (configuration.GetValue("Discovery:Enabled", true))
        {
            browser.Changed += OnChangedAsync;
            settings.Changed += ApplyAsync;
            Apply(settings.Current);
        }

        return Task.CompletedTask;
    }

    public Task StopAsync(CancellationToken cancellationToken)
    {
        settings.Changed -= ApplyAsync;
        browser.Changed -= OnChangedAsync;
        browser.Stop();
        return Task.CompletedTask;
    }

    private Task ApplyAsync(KoppelSettings value)
    {
        Apply(value);
        return Task.CompletedTask;
    }

    private void Apply(KoppelSettings value)
    {
        try
        {
            if (value.Role == KoppelRole.Server)
            {
                browser.Stop();
            }
            else
            {
                browser.Start();
            }
        }
        catch (SocketException ex)
        {
            LogBrowseFailed(ex);
        }
    }

    /// <summary>The browser raises one list at a time, in order, and waits for this, so a push cannot overtake an earlier one.</summary>
    private Task OnChangedAsync(IReadOnlyList<SkakelDiscoveredServer> servers) => Changed.RaiseAsync(Others(servers), logger);

    private List<DiscoveredServer> Others(IReadOnlyList<SkakelDiscoveredServer> servers) =>
        [.. servers.Where(s => !Fingerprints.AreEqual(s.Fingerprint, keys.Fingerprint)).Select(DiscoveredServer.From)];

    [LoggerMessage(Level = LogLevel.Warning, Message = "Could not look for servers on the local network; enter the server's address instead")]
    private partial void LogBrowseFailed(Exception exception);
}
