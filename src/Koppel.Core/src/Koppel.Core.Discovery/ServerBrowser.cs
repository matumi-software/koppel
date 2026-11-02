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
/// keeps is bounded: a fixed number of entries with a share per source address, fingerprints that
/// are really fingerprints, short names, and entries that go when nobody refreshes them. Entries
/// are told apart by instance name and fingerprint, so a server that copies a name sits beside the
/// real one instead of replacing it, and the UI can warn about the two.
/// </summary>
public sealed partial class ServerBrowser : IDisposable
{
    public const int MaxEntries = 64;
    public const int MaxEntriesPerAddress = 8;
    public const int MaxNameLength = 63;
    public const int MaxVersionLength = 32;

    /// <summary>An entry nobody refreshed for this long goes. The browse restarts well within it, which makes every live server answer again.</summary>
    public static readonly TimeSpan EntryLifetime = TimeSpan.FromMinutes(10);
    public static readonly TimeSpan RefreshEvery = TimeSpan.FromMinutes(3);
    public static readonly TimeSpan SweepEvery = TimeSpan.FromMinutes(1);

    private readonly ServiceBrowser _browser = new();
    private readonly IBrowseControl _control;
    private readonly Dictionary<EntryKey, Seen> _seen = [];
    private IReadOnlyList<DiscoveredServer> _published = [];
    private readonly Lock _gate = new();
    // Held around every start and stop of the browse. The browser's own threads take _gate, never this, so a stop can wait for them.
    private readonly Lock _browseLock = new();
    private readonly TimeProvider _time;
    private readonly ILogger _logger;
    // Only the latest list matters to anyone listening, so a slow listener skips the ones in between.
    private readonly Channel<IReadOnlyList<DiscoveredServer>> _changes = Channel.CreateBounded<IReadOnlyList<DiscoveredServer>>(
        new BoundedChannelOptions(1) { FullMode = BoundedChannelFullMode.DropOldest, SingleReader = true });
    private readonly ITimer _sweeper;
    private readonly Task _raising;
    private DateTimeOffset _browseStarted;
    private bool _wanted;

    public ServerBrowser(TimeProvider? time = null, ILogger<ServerBrowser>? logger = null)
        : this(time, logger, null)
    {
    }

    internal ServerBrowser(TimeProvider? time, ILogger<ServerBrowser>? logger, IBrowseControl? control)
    {
        _time = time ?? TimeProvider.System;
        _logger = logger ?? NullLogger<ServerBrowser>.Instance;
        _control = control ?? new ServiceBrowserControl(_browser);
        _browser.ServiceAdded += (_, e) => Update(e.Announcement);
        _browser.ServiceChanged += (_, e) => Update(e.Announcement);
        _browser.ServiceRemoved += (_, e) => Remove(e.Announcement);
        _raising = Task.Run(RaiseInOrderAsync);
        _sweeper = _time.CreateTimer(_ => SweepSafely(), null, SweepEvery, SweepEvery);
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

    public bool IsBrowsing => _control.IsBrowsing;

    public void Start()
    {
        lock (_browseLock)
        {
            _wanted = true;
            if (!_control.IsBrowsing)
            {
                lock (_gate)
                {
                    _browseStarted = _time.GetUtcNow();
                }

                _control.StartBrowse();
            }
        }
    }

    public void Stop()
    {
        lock (_browseLock)
        {
            _wanted = false;
            if (_control.IsBrowsing)
            {
                _control.StopBrowse();
            }
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
            Add(announcement.Instance, announcement.NetworkInterface?.Id, server, Rank(announcement.NetworkInterface));
        }
    }

    /// <summary>
    /// Keeps or refreshes an entry. When one address already holds its share, its stalest entry goes to
    /// make room. When the whole list is full, the stalest entry of the address holding the most goes,
    /// so a flood from one address pushes out its own entries and not other servers'.
    /// </summary>
    internal void Add(string instance, string? networkInterface, DiscoveredServer server, int rank)
    {
        var key = new EntryKey(instance, server.Fingerprint, networkInterface);
        lock (_gate)
        {
            if (!_seen.ContainsKey(key))
            {
                if (_seen.Count(e => e.Value.Server.Address == server.Address) >= MaxEntriesPerAddress)
                {
                    EvictStalest(server.Address);
                }
                else if (_seen.Count >= MaxEntries)
                {
                    EvictStalest(_seen.GroupBy(e => e.Value.Server.Address).MaxBy(g => g.Count())!.Key);
                }
            }

            _seen[key] = new Seen(instance, networkInterface, server, rank, _time.GetUtcNow());
            Publish();
        }
    }

    /// <summary>Drops the entry from this address that nobody has refreshed for longest, with the gate held.</summary>
    private void EvictStalest(string address)
    {
        var stalest = _seen.Where(e => e.Value.Server.Address == address).MinBy(e => e.Value.LastSeen).Key;
        _seen.Remove(stalest);
    }

    private void Remove(ServiceAnnouncement announcement)
    {
        // A goodbye may not carry the record, so without a fingerprint it removes every entry of that instance on that interface.
        var fingerprint = Read(announcement.Txt, announcement.Addresses, announcement.Port, announcement.NetworkInterface)?.Fingerprint;
        lock (_gate)
        {
            var gone = _seen.Where(e => e.Value.Instance == announcement.Instance
                && e.Value.Interface == announcement.NetworkInterface?.Id
                && (fingerprint is null || e.Key.Fingerprint == fingerprint)).Select(e => e.Key).ToList();
            foreach (var key in gone)
            {
                _seen.Remove(key);
            }

            if (gone.Count > 0)
            {
                Publish();
            }
        }
    }

    /// <summary>Drops what nobody refreshed, and restarts the browse now and then so live servers answer again.</summary>
    internal void Sweep()
    {
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

        }

        // Outside the gate: the browser raises its events on its own threads, and they take the gate.
        // Under the same lock as Start and Stop, so a role change cannot slip in between the check and the restart.
        lock (_browseLock)
        {
            if (!_wanted || !_control.IsBrowsing)
            {
                return;
            }

            var now = _time.GetUtcNow();
            lock (_gate)
            {
                if (_browseStarted + RefreshEvery > now)
                {
                    return;
                }

                _browseStarted = now;
            }

            _control.StopBrowse();
            _control.StartBrowse();
        }
    }

    /// <summary>The timer's callback. An exception that leaves a timer thread ends the process, so nothing leaves.</summary>
    private void SweepSafely()
    {
        try
        {
            Sweep();
        }
        catch (Exception ex)
        {
            LogSweepFailed(ex);
        }
    }

    private IReadOnlyList<DiscoveredServer> Snapshot() =>
        [.. _seen.Values
            .GroupBy(s => s.Server.Fingerprint)
            .Select(g => g.OrderBy(s => s.Rank).First().Server)
            .OrderBy(s => s.Name, StringComparer.OrdinalIgnoreCase)];

    /// <summary>Queues the current list when it differs from the last one queued, with the gate held, so lists leave in the order the changes happened.</summary>
    private void Publish()
    {
        var snapshot = Snapshot();
        if (!snapshot.SequenceEqual(_published))
        {
            _published = snapshot;
            _changes.Writer.TryWrite(snapshot);
        }
    }

    private async Task RaiseInOrderAsync()
    {
        await foreach (var servers in _changes.Reader.ReadAllAsync())
        {
            await Changed.RaiseAsync(servers, _logger);
        }
    }

    /// <summary>Prefers wired and wireless interfaces over virtual ones, such as Hyper-V switches.</summary>
    private static int Rank(NetworkInterface? arrivedOn) =>
        arrivedOn?.NetworkInterfaceType is NetworkInterfaceType.Ethernet or NetworkInterfaceType.Wireless80211
        && !arrivedOn.Description.Contains("Virtual", StringComparison.OrdinalIgnoreCase)
        && !arrivedOn.Description.Contains("Hyper-V", StringComparison.OrdinalIgnoreCase)
            ? 0
            : 1;

    [LoggerMessage(Level = LogLevel.Warning, Message = "Could not sweep the list of servers or restart the browse")]
    private partial void LogSweepFailed(Exception exception);

    /// <summary>One announcement: the instance name and fingerprint tell servers apart, the interface tells the copies of one server apart.</summary>
    private readonly record struct EntryKey(string Instance, string Fingerprint, string? Interface);

    private sealed record Seen(string Instance, string? Interface, DiscoveredServer Server, int Rank, DateTimeOffset LastSeen);

    /// <summary>Starting and stopping the browse, so a test can stand in for the network.</summary>
    internal interface IBrowseControl
    {
        bool IsBrowsing { get; }

        void StartBrowse();

        void StopBrowse();
    }

    private sealed class ServiceBrowserControl(ServiceBrowser browser) : IBrowseControl
    {
        public bool IsBrowsing => browser.IsBrowsing;

        public void StartBrowse() => browser.StartBrowse(KoppelService.Type, useSynchronizationContext: false);

        public void StopBrowse() => browser.StopBrowse();
    }
}
