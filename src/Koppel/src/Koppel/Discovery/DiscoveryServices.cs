using System.Net.Sockets;
using Koppel.Core.Configuration;
using Koppel.Core.Discovery;
using Koppel.Server;
using Koppel.Settings;

namespace Koppel.Discovery;

/// <summary>
/// Server role: announces this server on the local network so clients can find it, and stops when
/// the machine becomes a client only. <c>Discovery:Enabled=false</c> turns it off.
/// </summary>
internal sealed partial class ServerAnnouncerService(
    ServerAnnouncer announcer,
    SettingsStore settings,
    ServerSecretsStore secrets,
    HostInfo host,
    IConfiguration configuration,
    ILogger<ServerAnnouncerService> logger) : IHostedService
{
    private bool _enabled;

    public Task StartAsync(CancellationToken cancellationToken)
    {
        _enabled = configuration.GetValue("Discovery:Enabled", true);
        if (_enabled)
        {
            settings.Changed += ApplyAsync;
            Apply(settings.Current);
        }

        return Task.CompletedTask;
    }

    public Task StopAsync(CancellationToken cancellationToken)
    {
        settings.Changed -= ApplyAsync;
        announcer.Stop();
        return Task.CompletedTask;
    }

    private Task ApplyAsync(KoppelSettings value)
    {
        Apply(value);
        return Task.CompletedTask;
    }

    private void Apply(KoppelSettings value)
    {
        var shouldAnnounce = value.Role != KoppelRole.Client;
        if (shouldAnnounce == announcer.IsAnnouncing)
        {
            return;
        }

        try
        {
            if (shouldAnnounce)
            {
                announcer.Announce(new ServerAnnouncement(Environment.MachineName, host.NetworkPort, secrets.Fingerprint, SystemEndpoints.Version), host.DiscoveryPort);
            }
            else
            {
                announcer.Stop();
            }
        }
        catch (SocketException ex)
        {
            // No multicast on this network, or another responder holds the port exclusively. Pairing by address still works.
            LogAnnounceFailed(ex);
        }
    }

    [LoggerMessage(Level = LogLevel.Warning, Message = "Could not announce on the local network; clients must enter this server's address")]
    private partial void LogAnnounceFailed(Exception exception);
}

/// <summary>
/// Client role: finds servers on the local network while this machine can be a client, leaving out
/// itself. <c>Discovery:Enabled=false</c> turns it off.
/// </summary>
internal sealed partial class ServerFinder(
    ServerBrowser browser,
    SettingsStore settings,
    ServerSecretsStore secrets,
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

    /// <summary>The browser calls one list at a time, in order, and waits for this, so a push cannot overtake an earlier one.</summary>
    private Task OnChangedAsync(IReadOnlyList<DiscoveredServer> servers) => Changed.RaiseAsync(Others(servers), logger);

    private IReadOnlyList<DiscoveredServer> Others(IReadOnlyList<DiscoveredServer> servers) =>
        [.. servers.Where(s => !string.Equals(s.Fingerprint, secrets.Fingerprint, StringComparison.OrdinalIgnoreCase))];

    [LoggerMessage(Level = LogLevel.Warning, Message = "Could not look for servers on the local network; enter the server's address instead")]
    private partial void LogBrowseFailed(Exception exception);
}
