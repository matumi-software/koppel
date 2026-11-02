using System.Text.RegularExpressions;
using Makaretu.Dns;
using Microsoft.Extensions.Logging;

namespace Koppel.Core.Discovery;

/// <summary>
/// Announces this server on the local network with multicast DNS, and answers clients that ask.
/// Built on Makaretu.Dns.Multicast.New (MIT).
/// </summary>
public sealed partial class ServerAnnouncer(ILogger<ServerAnnouncer> logger) : IDisposable
{
    private readonly Lock _lock = new();
    private MulticastService? _mdns;
    private ServiceDiscovery? _discovery;

    public bool IsAnnouncing
    {
        get
        {
            lock (_lock)
            {
                return _discovery is not null;
            }
        }
    }

    /// <summary>Starts announcing, or replaces what is announced.</summary>
    public void Announce(ServerAnnouncement announcement)
    {
        ArgumentNullException.ThrowIfNull(announcement);
        lock (_lock)
        {
            StopLocked();
            var profile = new ServiceProfile(
                new DomainName(InstanceName(announcement.Name)),
                new DomainName(KoppelService.Type),
                checked((ushort)announcement.HttpsPort));
            profile.AddProperty(KoppelService.ProtocolKey, KoppelService.Protocol);
            profile.AddProperty(KoppelService.NameKey, announcement.Name);
            profile.AddProperty(KoppelService.FingerprintKey, announcement.Fingerprint);
            profile.AddProperty(KoppelService.VersionKey, announcement.Version);

            _mdns = new MulticastService();
            _discovery = new ServiceDiscovery(_mdns);
            _mdns.Start();
            _discovery.Advertise(profile);
            _discovery.Announce(profile);
            LogAnnouncing(announcement.Name, announcement.HttpsPort);
        }
    }

    /// <summary>Says goodbye on the network and stops answering.</summary>
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
        if (_discovery is null)
        {
            return;
        }

        _discovery.Unadvertise();
        _discovery.Dispose();
        _mdns?.Stop();
        _mdns?.Dispose();
        _discovery = null;
        _mdns = null;
        LogStopped();
    }

    /// <summary>A DNS label: letters, digits and hyphens, at most 63 characters.</summary>
    private static string InstanceName(string name)
    {
        var label = NotLabel().Replace(name, "-").Trim('-');
        return label.Length == 0 ? "koppel" : label[..Math.Min(label.Length, 63)];
    }

    [GeneratedRegex("[^A-Za-z0-9-]+")]
    private static partial Regex NotLabel();

    [LoggerMessage(Level = LogLevel.Information, Message = "Announcing {Name} on the local network (HTTPS port {Port})")]
    private partial void LogAnnouncing(string name, int port);

    [LoggerMessage(Level = LogLevel.Information, Message = "Stopped announcing on the local network")]
    private partial void LogStopped();
}
