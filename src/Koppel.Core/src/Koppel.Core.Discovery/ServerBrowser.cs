using System.Net;
using System.Net.NetworkInformation;
using System.Net.Sockets;
using System.Threading.Channels;
using Koppel.Core.Configuration;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Logging.Abstractions;
using Tmds.MDns;

namespace Koppel.Core.Discovery;

/// <summary>
/// Finds Koppel servers on the local network with multicast DNS and keeps the list current as they
/// come and go. Built on Tmds.MDns (MIT). Anyone on the network can announce anything, so what it
/// keeps is bounded: a fixed number of entries, fingerprints that are really fingerprints, short
/// names, and entries that go when nobody refreshes them.
/// </summary>
public sealed partial class ServerBrowser : IDisposable
{
    public const int MaxEntries = 64;
    public const int MaxNameLength = 63;
    public const int MaxVersionLength = 32;

    /// <summary>An entry nobody refreshed for this long goes. The browse restarts well within it, which makes every live server answer again.</summary>
    public static readonly TimeSpan EntryLifetime = TimeSpan.FromMinutes(10);
    public static readonly TimeSpan RefreshEvery = TimeSpan.FromMinutes(3);
    public static readonly TimeSpan SweepEvery = TimeSpan.FromMinutes(1);

    private readonly ServiceBrowser _browser = new();
    private readonly Dictionary<string, Seen> _seen = [];
    private readonly Lock _gate = new();
    private readonly TimeProvider _time;
    private readonly ILogger _logger;
    private readonly Channel<IReadOnlyList<DiscoveredServer>> _changes = Channel.CreateUnbounded<IReadOnlyList<DiscoveredServer>>(new UnboundedChannelOptions { SingleReader = true });
    private readonly ITimer _sweeper;
    private readonly Task _raising;
    private DateTimeOffset _browseStarted;

    public ServerBrowser(TimeProvider? time = null, ILogger<ServerBrowser>? logger = null)
    {
        _time = time ?? TimeProvider.System;
        _logger = logger ?? NullLogger<ServerBrowser>.Instance;
        _browser.ServiceAdded += (_, e) => Update(e.Announcement);
        _browser.ServiceChanged += (_, e) => Update(e.Announcement);
        _browser.ServiceRemoved += (_, e) => Remove(e.Announcement);
        _raising = Task.Run(RaiseInOrderAsync);
        _sweeper = _time.CreateTimer(_ => Sweep(), null, SweepEvery, SweepEvery);
    }

    /// <summary>Raised when a server appears, changes or goes away. One list at a time, in the order the changes happened.</summary>
    public event Func<IReadOnlyList<DiscoveredServer>, Task>? Changed;

    /// <summary>One entry per server, however many network interfaces it was seen on.</summary>
    public IReadOnlyList<DiscoveredServer> Servers
    {
        get
        {
            lock (_gate)
            {
                return Snapshot();
            }
        }
    }

    public bool IsBrowsing => _browser.IsBrowsing;

    public void Start()
    {
        if (!_browser.IsBrowsing)
        {
            lock (_gate)
            {
                _browseStarted = _time.GetUtcNow();
            }

            _browser.StartBrowse(KoppelService.Type, useSynchronizationContext: false);
        }
    }

    public void Stop()
    {
        if (_browser.IsBrowsing)
        {
            _browser.StopBrowse();
        }

        lock (_gate)
        {
            _seen.Clear();
            Publish();
        }
    }

    public void Dispose()
    {
        Stop();
        _sweeper.Dispose();
        _changes.Writer.TryComplete();
        _raising.GetAwaiter().GetResult();
    }

    /// <summary>Reads a server out of an announcement, or null when it is not a Koppel server we understand.</summary>
    public static DiscoveredServer? Read(IEnumerable<string> txt, IEnumerable<IPAddress> addresses, int port, NetworkInterface? arrivedOn = null)
    {
        var values = (txt ?? [])
            .Select(entry => entry.Split('=', 2))
            .Where(parts => parts.Length == 2)
            .GroupBy(parts => parts[0], StringComparer.OrdinalIgnoreCase)
            .ToDictionary(g => g.Key, g => g.First()[1], StringComparer.OrdinalIgnoreCase);
        if (values.GetValueOrDefault(KoppelService.ProtocolKey) != KoppelService.Protocol
            || NormalizeFingerprint(values.GetValueOrDefault(KoppelService.FingerprintKey)) is not { } fingerprint
            || PickAddress(addresses, arrivedOn) is not { } address)
        {
            return null;
        }

        return new DiscoveredServer(
            Bound(values.GetValueOrDefault(KoppelService.NameKey), MaxNameLength) ?? address.ToString(),
            address.ToString(),
            port,
            fingerprint,
            Bound(values.GetValueOrDefault(KoppelService.VersionKey), MaxVersionLength));
    }

    /// <summary>
    /// The fingerprint as upper-case hex pairs joined by colons, the form the server's Clients page shows,
    /// or null unless the text is exactly 64 hex digits, with or without colons.
    /// </summary>
    public static string? NormalizeFingerprint(string? text)
    {
        if (text is null || text.Any(c => c != ':' && !Uri.IsHexDigit(c)))
        {
            return null;
        }

        var hex = text.Replace(":", "", StringComparison.Ordinal).ToUpperInvariant();
        return hex.Length == 64 ? string.Join(':', Enumerable.Range(0, 32).Select(i => hex.Substring(i * 2, 2))) : null;
    }

    /// <summary>Text from the network, without control characters and cut to length. Null when nothing is left.</summary>
    private static string? Bound(string? text, int maxLength)
    {
        var clean = new string((text ?? "").Where(c => !char.IsControl(c)).ToArray()).Trim();
        if (clean.Length > maxLength)
        {
            clean = clean[..maxLength];
        }

        return clean.Length == 0 ? null : clean;
    }

    /// <summary>
    /// An IPv4 address on the subnet the announcement arrived on, when there is one. A server in a
    /// container on the host network also announces Docker's bridge addresses, which a client cannot reach.
    /// </summary>
    private static IPAddress? PickAddress(IEnumerable<IPAddress> addresses, NetworkInterface? arrivedOn)
    {
        var candidates = (addresses ?? []).Where(a => !IPAddress.IsLoopback(a)).ToList();
        var local = arrivedOn?.GetIPProperties().UnicastAddresses
            .Where(u => u.Address.AddressFamily == AddressFamily.InterNetwork)
            .ToList() ?? [];
        return candidates.FirstOrDefault(a => a.AddressFamily == AddressFamily.InterNetwork && local.Any(u => SameSubnet(a, u)))
            ?? candidates.FirstOrDefault(a => a.AddressFamily == AddressFamily.InterNetwork)
            ?? candidates.FirstOrDefault(a => a.AddressFamily == AddressFamily.InterNetworkV6 && !a.IsIPv6LinkLocal)
            ?? candidates.FirstOrDefault();
    }

    private static bool SameSubnet(IPAddress address, UnicastIPAddressInformation local)
    {
        var a = address.GetAddressBytes();
        var b = local.Address.GetAddressBytes();
        var mask = local.IPv4Mask.GetAddressBytes();
        return a.Length == 4 && Enumerable.Range(0, 4).All(i => (a[i] & mask[i]) == (b[i] & mask[i]));
    }

    private void Update(ServiceAnnouncement announcement)
    {
        if (Read(announcement.Txt, announcement.Addresses, announcement.Port, announcement.NetworkInterface) is { } server)
        {
            Add(Key(announcement), server, Rank(announcement.NetworkInterface));
        }
    }

    /// <summary>Keeps or refreshes an entry. A new one is turned away when the list is full.</summary>
    internal void Add(string key, DiscoveredServer server, int rank)
    {
        lock (_gate)
        {
            if (!_seen.ContainsKey(key) && _seen.Count >= MaxEntries)
            {
                return;
            }

            _seen[key] = new Seen(server, rank, _time.GetUtcNow());
            Publish();
        }
    }

    private void Remove(ServiceAnnouncement announcement)
    {
        lock (_gate)
        {
            if (_seen.Remove(Key(announcement)))
            {
                Publish();
            }
        }
    }

    /// <summary>Drops what nobody refreshed, and restarts the browse now and then so live servers answer again.</summary>
    internal void Sweep()
    {
        bool restart;
        lock (_gate)
        {
            var now = _time.GetUtcNow();
            var gone = _seen.Where(e => e.Value.LastSeen + EntryLifetime <= now).Select(e => e.Key).ToList();
            foreach (var key in gone)
            {
                _seen.Remove(key);
            }

            if (gone.Count > 0)
            {
                Publish();
            }

            restart = _browser.IsBrowsing && _browseStarted + RefreshEvery <= now;
            if (restart)
            {
                _browseStarted = now;
            }
        }

        // Outside the gate: the browser raises its events on its own threads, and they take the gate.
        if (restart)
        {
            try
            {
                _browser.StopBrowse();
                _browser.StartBrowse(KoppelService.Type, useSynchronizationContext: false);
            }
            catch (SocketException ex)
            {
                LogRestartFailed(ex);
            }
        }
    }

    private IReadOnlyList<DiscoveredServer> Snapshot() =>
        [.. _seen.Values
            .GroupBy(s => s.Server.Fingerprint)
            .Select(g => g.OrderBy(s => s.Rank).First().Server)
            .OrderBy(s => s.Name, StringComparer.OrdinalIgnoreCase)];

    /// <summary>Queues the current list, with the gate held, so lists leave in the order the changes happened.</summary>
    private void Publish() => _changes.Writer.TryWrite(Snapshot());

    private async Task RaiseInOrderAsync()
    {
        await foreach (var servers in _changes.Reader.ReadAllAsync())
        {
            await Changed.RaiseAsync(servers, _logger);
        }
    }

    private static string Key(ServiceAnnouncement a) => $"{a.Instance}|{a.NetworkInterface?.Id}";

    /// <summary>Prefers wired and wireless interfaces over virtual ones, such as Hyper-V switches.</summary>
    private static int Rank(NetworkInterface? arrivedOn) =>
        arrivedOn?.NetworkInterfaceType is NetworkInterfaceType.Ethernet or NetworkInterfaceType.Wireless80211
        && !arrivedOn.Description.Contains("Virtual", StringComparison.OrdinalIgnoreCase)
        && !arrivedOn.Description.Contains("Hyper-V", StringComparison.OrdinalIgnoreCase)
            ? 0
            : 1;

    [LoggerMessage(Level = LogLevel.Warning, Message = "Could not restart the browse for servers")]
    private partial void LogRestartFailed(Exception exception);

    private sealed record Seen(DiscoveredServer Server, int Rank, DateTimeOffset LastSeen);
}
