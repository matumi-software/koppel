using System.Collections.Concurrent;
using System.Net;
using System.Net.NetworkInformation;
using System.Net.Sockets;
using Tmds.MDns;

namespace Koppel.Core.Discovery;

/// <summary>
/// Finds Koppel servers on the local network with multicast DNS and keeps the list current as they
/// come and go. Built on Tmds.MDns (MIT).
/// </summary>
public sealed class ServerBrowser : IDisposable
{
    private readonly ServiceBrowser _browser = new();
    private readonly ConcurrentDictionary<string, Seen> _seen = new();

    public ServerBrowser()
    {
        _browser.ServiceAdded += (_, e) => Update(e.Announcement);
        _browser.ServiceChanged += (_, e) => Update(e.Announcement);
        _browser.ServiceRemoved += (_, e) => Remove(e.Announcement);
    }

    /// <summary>Raised when a server appears, changes or goes away.</summary>
    public event Action<IReadOnlyList<DiscoveredServer>>? Changed;

    /// <summary>One entry per server, however many network interfaces it was seen on.</summary>
    public IReadOnlyList<DiscoveredServer> Servers =>
        [.. _seen.Values
            .GroupBy(s => s.Server.Fingerprint)
            .Select(g => g.OrderBy(s => s.Rank).First().Server)
            .OrderBy(s => s.Name, StringComparer.OrdinalIgnoreCase)];

    public bool IsBrowsing => _browser.IsBrowsing;

    public void Start()
    {
        if (!_browser.IsBrowsing)
        {
            _browser.StartBrowse(KoppelService.Type, useSynchronizationContext: false);
        }
    }

    public void Stop()
    {
        if (_browser.IsBrowsing)
        {
            _browser.StopBrowse();
        }

        _seen.Clear();
        Changed?.Invoke([]);
    }

    public void Dispose() => Stop();

    /// <summary>Reads a server out of an announcement, or null when it is not a Koppel server we understand.</summary>
    public static DiscoveredServer? Read(IEnumerable<string> txt, IEnumerable<IPAddress> addresses, int port, NetworkInterface? arrivedOn = null)
    {
        var values = (txt ?? [])
            .Select(entry => entry.Split('=', 2))
            .Where(parts => parts.Length == 2)
            .GroupBy(parts => parts[0], StringComparer.OrdinalIgnoreCase)
            .ToDictionary(g => g.Key, g => g.First()[1], StringComparer.OrdinalIgnoreCase);
        if (values.GetValueOrDefault(KoppelService.ProtocolKey) != KoppelService.Protocol
            || values.GetValueOrDefault(KoppelService.FingerprintKey) is not { Length: > 0 } fingerprint
            || PickAddress(addresses, arrivedOn) is not { } address)
        {
            return null;
        }

        return new DiscoveredServer(
            values.GetValueOrDefault(KoppelService.NameKey) ?? address.ToString(),
            address.ToString(),
            port,
            fingerprint,
            values.GetValueOrDefault(KoppelService.VersionKey));
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
            _seen[Key(announcement)] = new Seen(server, Rank(announcement.NetworkInterface));
            Changed?.Invoke(Servers);
        }
    }

    private void Remove(ServiceAnnouncement announcement)
    {
        if (_seen.TryRemove(Key(announcement), out _))
        {
            Changed?.Invoke(Servers);
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

    private sealed record Seen(DiscoveredServer Server, int Rank);
}
