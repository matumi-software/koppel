using System.Net;
using System.Net.NetworkInformation;
using System.Net.Sockets;

namespace Koppel.Web;

/// <summary>Who may use the UI.</summary>
internal enum UiReach
{
    /// <summary>This machine only.</summary>
    Local,

    /// <summary>This machine and the local network.</summary>
    Lan,

    /// <summary>Anyone who can reach the port.</summary>
    Any,
}

/// <summary>
/// Decides by the caller's address whether it may use the UI, from <c>Ui:AllowFrom</c>, as Sunshine's
/// <c>origin_web_ui_allowed</c> does. The UI shares its port with everything other Koppel machines do,
/// so the port cannot keep it private; this can. The network surfaces are not affected: they have tokens.
/// </summary>
/// <remarks>
/// The local network is loopback, the private IPv4 ranges, link-local, IPv6 unique local addresses, and
/// any network this machine has an address on (most home IPv6 is global, so the private ranges alone
/// would turn a neighbour away). Carrier-grade NAT (100.64.0.0/10) is not local.
/// </remarks>
internal sealed class UiAccess(UiReach reach, Func<IReadOnlyList<(IPAddress Network, int PrefixLength)>>? localNetworks = null)
{
    public const string Setting = "Ui:AllowFrom";

    private static readonly (IPAddress Network, int PrefixLength)[] PrivateRanges =
    [
        (IPAddress.Parse("10.0.0.0"), 8),
        (IPAddress.Parse("172.16.0.0"), 12),
        (IPAddress.Parse("192.168.0.0"), 16),
        (IPAddress.Parse("169.254.0.0"), 16),
        (IPAddress.Parse("fc00::"), 7),
        (IPAddress.Parse("fe80::"), 10),
    ];

    private readonly Func<IReadOnlyList<(IPAddress Network, int PrefixLength)>> _localNetworks = localNetworks ?? CachedInterfaceNetworks;

    public UiReach Reach => reach;

    /// <summary>
    /// The setting is <c>local</c>, <c>lan</c> or <c>any</c>. Unset, it is <c>lan</c>, or <c>local</c> in
    /// Development. Anything else stops the start, naming the setting.
    /// </summary>
    public static UiReach Resolve(string? setting, bool development)
    {
        if (string.IsNullOrWhiteSpace(setting))
        {
            return development ? UiReach.Local : UiReach.Lan;
        }

        return setting.Trim().ToLowerInvariant() switch
        {
            "local" => UiReach.Local,
            "lan" => UiReach.Lan,
            "any" => UiReach.Any,
            _ => throw new InvalidOperationException($"{Setting} is \"{setting}\". Use local (this machine), lan (this machine and the local network) or any."),
        };
    }

    /// <summary>True when a caller at this address may use the UI. No address means no network connection, such as an in-process test server.</summary>
    public bool Allows(IPAddress? remote)
    {
        if (remote is null || reach == UiReach.Any)
        {
            return true;
        }

        if (remote.IsIPv4MappedToIPv6)
        {
            remote = remote.MapToIPv4();
        }

        if (IPAddress.IsLoopback(remote))
        {
            return true;
        }

        return reach == UiReach.Lan
            && (PrivateRanges.Any(range => InNetwork(remote, range.Network, range.PrefixLength))
                || _localNetworks().Any(range => InNetwork(remote, range.Network, range.PrefixLength)));
    }

    internal static bool InNetwork(IPAddress address, IPAddress network, int prefixLength)
    {
        if (address.AddressFamily != network.AddressFamily)
        {
            return false;
        }

        var a = address.GetAddressBytes();
        var n = network.GetAddressBytes();
        var whole = prefixLength / 8;
        if (!a.AsSpan(0, whole).SequenceEqual(n.AsSpan(0, whole)))
        {
            return false;
        }

        var rest = prefixLength % 8;
        if (rest == 0)
        {
            return true;
        }

        var mask = (byte)(0xFF << (8 - rest));
        return (a[whole] & mask) == (n[whole] & mask);
    }

    private static readonly Lock CacheLock = new();
    private static IReadOnlyList<(IPAddress Network, int PrefixLength)> _cached = [];
    private static long _cachedAt = long.MinValue;

    // Addresses change with DHCP and Wi-Fi, so the list is read again every 30 seconds rather than once.
    private static IReadOnlyList<(IPAddress Network, int PrefixLength)> CachedInterfaceNetworks()
    {
        lock (CacheLock)
        {
            var now = Environment.TickCount64;
            if (now - _cachedAt > 30_000)
            {
                _cached = InterfaceNetworks();
                _cachedAt = now;
            }

            return _cached;
        }
    }

    private static List<(IPAddress Network, int PrefixLength)> InterfaceNetworks()
    {
        try
        {
            return
            [
                .. NetworkInterface.GetAllNetworkInterfaces()
                    .Where(nic => nic.OperationalStatus == OperationalStatus.Up && nic.NetworkInterfaceType != NetworkInterfaceType.Loopback)
                    .SelectMany(nic => nic.GetIPProperties().UnicastAddresses)
                    // A very short prefix would make half the internet local; real local networks are /8 or longer, /48 or longer on IPv6.
                    .Where(unicast => unicast.Address.AddressFamily == AddressFamily.InterNetwork ? unicast.PrefixLength is >= 8 and <= 32 : unicast.PrefixLength is >= 48 and <= 128)
                    .Select(unicast => (unicast.Address, unicast.PrefixLength)),
            ];
        }
        catch (NetworkInformationException)
        {
            return [];
        }
    }
}
