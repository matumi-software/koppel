using System.Net;
using System.Net.NetworkInformation;
using System.Net.Sockets;
using System.Text.Json;
using System.Threading.Channels;
using Koppel.Core.Configuration;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Logging.Abstractions;

namespace Koppel.Core.Discovery;

/// <summary>
/// Finds Koppel servers on the local network: broadcasts a probe every few seconds on every network
/// this machine is on, and lists the servers that answer (see <see cref="KoppelService"/>). Anyone on
/// the network can answer anything, so what it keeps is bounded: a fixed number of entries with a
/// share per source address, fingerprints that are really fingerprints, short names, and entries that
/// go when they stop answering. Entries are told apart by fingerprint and address, so a server that
/// copies a name sits beside the real one instead of replacing it, and the UI can warn about the two.
/// </summary>
public sealed partial class ServerBrowser : IDisposable
{
    public const int MaxEntries = 64;
    public const int MaxEntriesPerAddress = 8;
    public const int MaxNameLength = 63;
    public const int MaxVersionLength = 32;

    /// <summary>How often a probe goes out while browsing, and expired entries are dropped.</summary>
    public static readonly TimeSpan ProbeEvery = TimeSpan.FromSeconds(10);

    private readonly IBrowseControl _control;
    private readonly TimeSpan _probeEvery;
    private readonly Dictionary<EntryKey, Seen> _seen = [];
    private IReadOnlyList<DiscoveredServer> _published = [];
    private readonly Lock _gate = new();
    // Held around every start, stop and probe. The control's own threads take _gate, never this, so a stop can wait for them.
    private readonly Lock _browseLock = new();
    private readonly TimeProvider _time;
    private readonly ILogger _logger;
    // Only the latest list matters to anyone listening, so a slow listener skips the ones in between.
    private readonly Channel<IReadOnlyList<DiscoveredServer>> _changes = Channel.CreateBounded<IReadOnlyList<DiscoveredServer>>(
        new BoundedChannelOptions(1) { FullMode = BoundedChannelFullMode.DropOldest, SingleReader = true });
    private readonly ITimer _prober;
    private readonly Task _raising;
    private bool _wanted;

    /// <param name="port">The UDP port servers answer on: Koppel's HTTPS port number unless changed.</param>
    public ServerBrowser(TimeProvider? time = null, ILogger<ServerBrowser>? logger = null, int port = KoppelService.DefaultPort)
        : this(time, logger, Udp(port))
    {
    }

    internal ServerBrowser(TimeProvider? time, ILogger<ServerBrowser>? logger, IBrowseControl control, TimeSpan? probeEvery = null)
    {
        _time = time ?? TimeProvider.System;
        _logger = logger ?? NullLogger<ServerBrowser>.Instance;
        _control = control;
        _probeEvery = probeEvery ?? ProbeEvery;
        _raising = Task.Run(RaiseInOrderAsync);
        _prober = _time.CreateTimer(_ => TickSafely(), null, _probeEvery, _probeEvery);
    }

    /// <summary>Raised when a server appears, changes or goes away. One list at a time, in the order the changes happened.</summary>
    public event Func<IReadOnlyList<DiscoveredServer>, Task>? Changed;

    /// <summary>An entry that missed three probes in a row, and a little more, is gone.</summary>
    public TimeSpan EntryLifetime => _probeEvery * 3.5;

    /// <summary>One entry per server, however many of its addresses answered.</summary>
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
                _control.StartBrowse(Add);
                _control.Probe();
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
        _prober.Dispose();
        (_control as IDisposable)?.Dispose();
        _changes.Writer.TryComplete();
        _raising.GetAwaiter().GetResult();
    }

    /// <summary>Reads a server out of an answer, or null when it is not a Koppel answer we understand.</summary>
    /// <param name="from">Where the answer came from, which is the address to reach the server on.</param>
    public static DiscoveredServer? Read(ReadOnlySpan<byte> answer, IPAddress from)
    {
        ArgumentNullException.ThrowIfNull(from);
        if (!answer.StartsWith(KoppelService.ReplyLine))
        {
            return null;
        }

        KoppelService.ReplyBody? body;
        try
        {
            body = JsonSerializer.Deserialize<KoppelService.ReplyBody>(answer[KoppelService.ReplyLine.Length..], KoppelService.Json);
        }
        catch (JsonException)
        {
            return null;
        }

        if (body is not { Port: >= 1 and <= 65535 } || NormalizeFingerprint(body.Fingerprint) is not { } fingerprint)
        {
            return null;
        }

        var address = (from.IsIPv4MappedToIPv6 ? from.MapToIPv4() : from).ToString();
        return new DiscoveredServer(
            Bound(body.Name, MaxNameLength) ?? address,
            address,
            body.Port,
            fingerprint,
            Bound(body.Version, MaxVersionLength));
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
    internal static string? Bound(string? text, int maxLength)
    {
        var clean = new string((text ?? "").Where(c => !char.IsControl(c)).ToArray()).Trim();
        if (clean.Length > maxLength)
        {
            clean = clean[..maxLength];
        }

        return clean.Length == 0 ? null : clean;
    }

    /// <summary>
    /// Keeps or refreshes an entry. When one address already holds its share, its stalest entry goes to
    /// make room. When the whole list is full, the stalest entry of the address holding the most goes,
    /// so a flood from one address pushes out its own entries and not other servers'.
    /// </summary>
    internal void Add(DiscoveredServer server, int rank)
    {
        var key = new EntryKey(server.Fingerprint, server.Address);
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

            _seen[key] = new Seen(server, rank, _time.GetUtcNow());
            Publish();
        }
    }

    /// <summary>Drops the entry from this address that answered longest ago, with the gate held.</summary>
    private void EvictStalest(string address)
    {
        var stalest = _seen.Where(e => e.Value.Server.Address == address).MinBy(e => e.Value.LastSeen).Key;
        _seen.Remove(stalest);
    }

    /// <summary>Drops the entries that stopped answering.</summary>
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
    }

    /// <summary>
    /// Drops what stopped answering, then probes again. Under the same lock as Start and Stop, so a role
    /// change cannot slip in between the check and the probe.
    /// </summary>
    private void Tick()
    {
        Sweep();
        lock (_browseLock)
        {
            if (_wanted && _control.IsBrowsing)
            {
                _control.Probe();
            }
        }
    }

    /// <summary>The timer's callback. An exception that leaves a timer thread ends the process, so nothing leaves.</summary>
    private void TickSafely()
    {
        try
        {
            Tick();
        }
        catch (Exception ex)
        {
            LogTickFailed(ex);
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

    /// <summary>The real network: probes to <paramref name="targets"/>, or to every network's broadcast address.</summary>
    internal static IBrowseControl Udp(int port, IReadOnlyList<IPEndPoint>? targets = null) => new UdpBrowse(port, targets);

    [LoggerMessage(Level = LogLevel.Warning, Message = "Could not sweep the list of servers or send a probe")]
    private partial void LogTickFailed(Exception exception);

    /// <summary>One server at one address: the fingerprint tells servers apart, the address tells the copies of one server apart.</summary>
    private readonly record struct EntryKey(string Fingerprint, string Address);

    private sealed record Seen(DiscoveredServer Server, int Rank, DateTimeOffset LastSeen);

    /// <summary>Starting and stopping the browse and sending probes, so a test can stand in for the network.</summary>
    internal interface IBrowseControl
    {
        bool IsBrowsing { get; }

        /// <summary>Starts listening for answers. <paramref name="found"/> gets each server and its rank (0 for a physical network).</summary>
        void StartBrowse(Action<DiscoveredServer, int> found);

        void StopBrowse();

        void Probe();
    }

    private sealed class UdpBrowse(int port, IReadOnlyList<IPEndPoint>? targets) : IBrowseControl, IDisposable
    {
        private readonly Lock _lock = new();
        private Socket? _socket;
        private CancellationTokenSource? _stop;

        public bool IsBrowsing
        {
            get
            {
                lock (_lock)
                {
                    return _socket is not null;
                }
            }
        }

        public void StartBrowse(Action<DiscoveredServer, int> found)
        {
            lock (_lock)
            {
                var socket = new Socket(AddressFamily.InterNetwork, SocketType.Dgram, ProtocolType.Udp) { EnableBroadcast = true };
                try
                {
                    socket.Bind(new IPEndPoint(IPAddress.Any, 0));
                }
                catch
                {
                    socket.Dispose();
                    throw;
                }

                _socket = socket;
                _stop = new CancellationTokenSource();
                _ = ListenAsync(socket, found, _stop.Token);
            }
        }

        public void StopBrowse()
        {
            lock (_lock)
            {
                _stop?.Cancel();
                _stop?.Dispose();
                _socket?.Dispose();
                _stop = null;
                _socket = null;
            }
        }

        public void Dispose() => StopBrowse();

        public void Probe()
        {
            Socket? socket;
            lock (_lock)
            {
                socket = _socket;
            }

            var probe = KoppelService.CreateProbe();
            foreach (var target in targets ?? BroadcastTargets(port))
            {
                try
                {
                    socket?.SendTo(probe, target);
                }
                catch (SocketException)
                {
                    // That network is down or refuses broadcasts; the others still get the probe.
                }
            }
        }

        private static async Task ListenAsync(Socket socket, Action<DiscoveredServer, int> found, CancellationToken cancellationToken)
        {
            var buffer = new byte[2048];
            var anyone = new IPEndPoint(IPAddress.Any, 0);
            while (!cancellationToken.IsCancellationRequested)
            {
                SocketReceiveFromResult received;
                try
                {
                    received = await socket.ReceiveFromAsync(buffer, SocketFlags.None, anyone, cancellationToken);
                }
                catch (Exception ex) when (ex is OperationCanceledException or ObjectDisposedException)
                {
                    return;
                }
                catch (SocketException)
                {
                    // Windows reports a probe nobody received on the next receive. Keep listening.
                    continue;
                }

                var from = ((IPEndPoint)received.RemoteEndPoint).Address;
                if (Read(buffer.AsSpan(0, received.ReceivedBytes), from) is { } server)
                {
                    found(server, Rank(from));
                }
            }
        }

        /// <summary>
        /// The limited broadcast address and each IPv4 network's own broadcast address. On a machine with
        /// several networks the limited one only leaves through one of them.
        /// </summary>
        private static IEnumerable<IPEndPoint> BroadcastTargets(int port) =>
            LocalNetworks()
                .Where(u => !u.IPv4Mask.Equals(IPAddress.Broadcast))
                .Select(u =>
                {
                    var address = u.Address.GetAddressBytes();
                    var mask = u.IPv4Mask.GetAddressBytes();
                    return new IPAddress([.. address.Select((b, i) => (byte)(b | ~mask[i]))]);
                })
                .Prepend(IPAddress.Broadcast)
                .Distinct()
                .Select(address => new IPEndPoint(address, port));

        private static IEnumerable<UnicastIPAddressInformation> LocalNetworks(Func<NetworkInterface, bool>? where = null) =>
            NetworkInterface.GetAllNetworkInterfaces()
                .Where(n => n.OperationalStatus == OperationalStatus.Up && n.NetworkInterfaceType != NetworkInterfaceType.Loopback)
                .Where(where ?? (_ => true))
                .SelectMany(n => n.GetIPProperties().UnicastAddresses)
                .Where(u => u.Address.AddressFamily == AddressFamily.InterNetwork);

        /// <summary>
        /// Prefers an address on a wired or wireless network over one on a virtual network, such as a
        /// Hyper-V switch or Docker's bridge, when a server answers on both.
        /// </summary>
        private static int Rank(IPAddress address)
        {
            var physical = LocalNetworks(n =>
                n.NetworkInterfaceType is NetworkInterfaceType.Ethernet or NetworkInterfaceType.Wireless80211
                && !n.Description.Contains("Virtual", StringComparison.OrdinalIgnoreCase)
                && !n.Description.Contains("Hyper-V", StringComparison.OrdinalIgnoreCase));
            return physical.Any(u => SameSubnet(address, u)) ? 0 : 1;
        }

        private static bool SameSubnet(IPAddress address, UnicastIPAddressInformation local)
        {
            var a = address.GetAddressBytes();
            var b = local.Address.GetAddressBytes();
            var mask = local.IPv4Mask.GetAddressBytes();
            return a.Length == 4 && Enumerable.Range(0, 4).All(i => (a[i] & mask[i]) == (b[i] & mask[i]));
        }
    }
}
