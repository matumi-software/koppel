using System.Net;
using System.Net.Sockets;
using Koppel.Core.Activity;
using Koppel.Core.Discovery;
using Koppel.Core.Security;
using Koppel.Discovery;
using Koppel.Usb;

namespace Koppel.Client;

/// <summary>
/// Follows the saved server to another address or port on the local network, as when it came back on 5240
/// after an upgrade from 5241, or its machine got a new IP address. While the link cannot reach the saved
/// address at all, and discovery hears a server with the pinned certificate somewhere else, the saved
/// address becomes that one. The fingerprint in a discovery answer is public, so anyone on the network can
/// copy it: before the move, the new address must prove it holds the pinned certificate over a pinned
/// handshake, so a copied answer cannot draw the client away from its server. A certificate that does not
/// match or credentials the server refused are not a move. With <c>Discovery:Enabled=false</c> nothing is
/// heard, so nothing moves.
/// </summary>
internal sealed partial class ServerMoves(
    ConnectionStore connection,
    IDiscoveredServers discovery,
    IActivityRecorder activity,
    TimeProvider time,
    ILogger<ServerMoves> logger) : IHostedService, IDisposable
{
    /// <summary>How long an address the client moved to is not moved to again, so two addresses that both fail cannot take turns.</summary>
    internal static readonly TimeSpan HoldOff = TimeSpan.FromMinutes(5);

    /// <summary>How long the saved host name may take to resolve, when working out whether a server is on the same host.</summary>
    private static readonly TimeSpan ResolveTimeout = TimeSpan.FromSeconds(5);

    /// <summary>How long an address has to prove it holds the pinned certificate.</summary>
    private static readonly TimeSpan ProofTimeout = TimeSpan.FromSeconds(5);

    private readonly Lock _gate = new();
    private readonly SemaphoreSlim _moving = new(1, 1);
    private readonly Dictionary<string, DateTimeOffset> _movedTo = new(StringComparer.OrdinalIgnoreCase);

    /// <summary>The connection whose server did not answer at its address the last time the link tried, or null.</summary>
    private ClientConnection? _stranded;

    public Task StartAsync(CancellationToken cancellationToken)
    {
        discovery.Changed += OnServersChangedAsync;
        return Task.CompletedTask;
    }

    public Task StopAsync(CancellationToken cancellationToken)
    {
        discovery.Changed -= OnServersChangedAsync;
        return Task.CompletedTask;
    }

    public void Dispose() => _moving.Dispose();

    /// <summary>
    /// The link to <paramref name="link"/> failed. When nothing answered at its address, the server is
    /// looked for among the ones discovery hears now, and again each time that list changes, until the link
    /// is up or fails for another reason.
    /// </summary>
    public Task LinkFailedAsync(ClientConnection link, Exception exception)
    {
        var unreachable = RemoteFailure.IsUnreachable(exception);
        lock (_gate)
        {
            _stranded = unreachable ? link : null;
        }

        return unreachable ? FollowAsync(link, discovery.Servers) : Task.CompletedTask;
    }

    /// <summary>The link is up, so the server is where the connection says.</summary>
    public void LinkUp()
    {
        lock (_gate)
        {
            _stranded = null;
        }
    }

    private Task OnServersChangedAsync(IReadOnlyList<DiscoveredServer> servers)
    {
        ClientConnection? stranded;
        lock (_gate)
        {
            stranded = _stranded;
        }

        return stranded is null ? Task.CompletedTask : FollowAsync(stranded, servers);
    }

    /// <summary>One move at a time, and only away from the connection that is still saved.</summary>
    private async Task FollowAsync(ClientConnection stranded, IReadOnlyList<DiscoveredServer> servers)
    {
        if (connection.Current != stranded || stranded.ServerUrl is not { } savedUrl || !Uri.TryCreate(savedUrl, UriKind.Absolute, out var saved))
        {
            return;
        }

        await _moving.WaitAsync();
        try
        {
            var now = time.GetUtcNow();
            foreach (var (url, at) in _movedTo.ToList())
            {
                if (now - at >= HoldOff)
                {
                    _movedTo.Remove(url);
                }
            }

            var candidates = servers.Where(s => Matches(stranded, s)).ToList();
            if (candidates.Count == 0)
            {
                return;
            }

            string? to = null;
            foreach (var url in Candidates(stranded, candidates, await AddressesOfAsync(saved), _movedTo.ContainsKey))
            {
                if (await HoldsCertificateAsync(stranded, url))
                {
                    to = url;
                    break;
                }
            }

            if (to is null || !await connection.MoveServerAsync(stranded, to, CancellationToken.None))
            {
                return;
            }

            _movedTo[to] = now;
            lock (_gate)
            {
                if (_stranded == stranded)
                {
                    _stranded = null;
                }
            }

            var was = saved.Authority;
            var moved = new Uri(to).Authority;
            LogMoved(was, moved);
            await activity.RecordAsync(new EntityRef("link", new Uri(to).Host, "link"), ActivityReason.ServerMoved, ActivityOutcome.Succeeded, $"Server moved from {was} to {moved}");
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        {
            // Nothing moved; the next failure or the next list tries again.
            LogMoveFailed(ex);
        }
        finally
        {
            _moving.Release();
        }
    }

    /// <summary>A server with the certificate this connection pins, or the next one it was told about.</summary>
    private static bool Matches(ClientConnection current, DiscoveredServer server) =>
        ConnectionUpdate.SameFingerprint(server.Fingerprint, current.ServerFingerprint)
        || (current.NextServerFingerprint is not null && ConnectionUpdate.SameFingerprint(server.Fingerprint, current.NextServerFingerprint));

    /// <summary>
    /// The addresses <paramref name="current"/> could move to, best first: where discovery hears a server
    /// with its certificate, other than the saved address. One on the saved host (<paramref name="savedHost"/>)
    /// on another port comes first, and keeps the saved host name; then any other. An address it moved to a
    /// little while ago is <paramref name="heldOff"/>.
    /// </summary>
    internal static IReadOnlyList<string> Candidates(ClientConnection current, IEnumerable<DiscoveredServer> servers, IReadOnlyCollection<IPAddress> savedHost, Func<string, bool>? heldOff = null)
    {
        var saved = new Uri(current.ServerUrl!);
        return servers
            .Where(s => Matches(current, s))
            .Select(s =>
            {
                var sameHost = IPAddress.TryParse(s.Address, out var address) && savedHost.Contains(Plain(address));
                var url = sameHost ? new UriBuilder(saved) { Port = s.HttpsPort }.Uri : new Uri(s.Url);
                return (SameHost: sameHost, Url: url.GetLeftPart(UriPartial.Authority));
            })
            .Where(c => !string.Equals(c.Url, saved.GetLeftPart(UriPartial.Authority), StringComparison.OrdinalIgnoreCase) && heldOff?.Invoke(c.Url) != true)
            .OrderByDescending(c => c.SameHost)
            .Select(c => c.Url)
            .Distinct(StringComparer.OrdinalIgnoreCase)
            .ToList();
    }

    /// <summary>
    /// Whether <paramref name="url"/> holds a certificate <paramref name="current"/> trusts: one request over a
    /// handler pinned to its fingerprints, with no token or secret. Any answer after the pinned handshake
    /// proves it, a 401 included; a handshake that fails or takes too long does not.
    /// </summary>
    private async Task<bool> HoldsCertificateAsync(ClientConnection current, string url)
    {
        using var handler = ServerCertificate.CreatePinnedHandler(current.Trusts);
        handler.AllowAutoRedirect = false;
        using var http = new HttpClient(handler) { Timeout = ProofTimeout };
        try
        {
            using var response = await http.GetAsync(new Uri(new Uri(url), "/api/remote/hello"), HttpCompletionOption.ResponseHeadersRead);
            return true;
        }
        catch (Exception ex) when (ex is HttpRequestException or TaskCanceledException)
        {
            LogNotProven(url, ex.Message);
            return false;
        }
    }

    /// <summary>The addresses the saved host stands for: itself when it is one, otherwise what its name resolves to now, or none.</summary>
    private async Task<IReadOnlyCollection<IPAddress>> AddressesOfAsync(Uri saved)
    {
        if (IPAddress.TryParse(saved.IdnHost, out var literal))
        {
            return [Plain(literal)];
        }

        using var timeout = new CancellationTokenSource(ResolveTimeout);
        try
        {
            return [.. (await Dns.GetHostAddressesAsync(saved.IdnHost, timeout.Token)).Select(Plain)];
        }
        catch (Exception ex) when (ex is SocketException or OperationCanceledException)
        {
            LogNotResolved(saved.IdnHost, ex.Message);
            return [];
        }
    }

    private static IPAddress Plain(IPAddress address) => address.IsIPv4MappedToIPv6 ? address.MapToIPv4() : address;

    [LoggerMessage(Level = LogLevel.Information, Message = "The server stopped answering at {OldAddress} and was found at {NewAddress}; the connection follows it")]
    private partial void LogMoved(string oldAddress, string newAddress);

    [LoggerMessage(Level = LogLevel.Information, Message = "Could not resolve {Host} to compare it with the servers on the network: {Reason}")]
    private partial void LogNotResolved(string host, string reason);

    [LoggerMessage(Level = LogLevel.Warning, Message = "A server with the pinned fingerprint was heard at {Url}, but it did not prove it holds the pinned certificate: {Reason}")]
    private partial void LogNotProven(string url, string reason);

    [LoggerMessage(Level = LogLevel.Warning, Message = "Could not follow the server to its new address")]
    private partial void LogMoveFailed(Exception exception);
}
