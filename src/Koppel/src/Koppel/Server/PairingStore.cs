using System.Collections.Concurrent;
using System.Net;
using System.Net.Sockets;
using System.Security.Cryptography;
using System.Text;
using Koppel.Core.Activity;
using Koppel.Core.Configuration;
using Koppel.Core.Security;

namespace Koppel.Server;

/// <summary>A pairing request as the server's UI sees it. The code and the client's poll token are not in it.</summary>
/// <param name="State"><c>waiting</c>, or <c>denied</c> for a short while after three wrong codes, so the UI can say why the request went.</param>
public sealed record PairingRequestView(string Id, string ClientName, string From, DateTimeOffset RequestedAt, DateTimeOffset ExpiresAt, string State = "waiting", string? Message = null);

/// <summary>What a client learns when it polls its request.</summary>
/// <param name="State"><c>waiting</c>, <c>approved</c> (with the credentials, once), <c>denied</c> or <c>expired</c>.</param>
internal sealed record PairingPoll(string State, string? ClientId = null, string? ClientSecret = null);

/// <summary>What the client gets back from opening a request. Only the opener ever sees the poll token.</summary>
internal sealed record PairingOpened(string Id, string ServerNonce, DateTimeOffset ExpiresAt, string PollToken);

/// <summary>
/// Server role: pairing requests from clients, kept in memory for a few minutes. A request becomes
/// visible in the UI only after the client revealed its nonce, so the code exists on both ends. The
/// person approves it by typing the code the client shows; three wrong codes drop the request.
/// </summary>
/// <remarks>
/// A man in the middle gets a one in a million chance per request, so what bounds it is how many
/// requests the server takes: a server-wide budget of opens per window that nothing refunds, a limit
/// per address, and a cap on requests a person has to look at. A request that never reveals its
/// nonce is dropped after a few seconds. Each request has its own gate, so approving, refusing and
/// counting wrong codes on one request cannot overlap.
/// </remarks>
internal sealed partial class PairingStore : IDisposable
{
    public static readonly TimeSpan Lifetime = TimeSpan.FromMinutes(5);
    public static readonly TimeSpan UnrevealedLifetime = TimeSpan.FromSeconds(30);
    public static readonly TimeSpan DeniedVisibleFor = TimeSpan.FromSeconds(15);
    public static readonly TimeSpan SweepInterval = TimeSpan.FromSeconds(5);
    public static readonly TimeSpan OpenWindow = TimeSpan.FromMinutes(5);
    public const int OpensPerWindow = 20;
    public const int MaxOpen = 10;
    public const int MaxOpenPerAddress = 3;
    public const int MaxWrongCodes = 3;

    private const string Waiting = "waiting";
    private const string Approved = "approved";
    private const string Denied = "denied";
    private const string Expired = "expired";

    private readonly ConcurrentDictionary<string, Request> _requests = new();
    private readonly Queue<DateTimeOffset> _opens = new();
    private readonly Lock _admission = new();
    private readonly ServerSecretsStore _secrets;
    private readonly IActivityRecorder _activity;
    private readonly TimeProvider _time;
    private readonly ILogger<PairingStore> _logger;
    private readonly ITimer _sweeper;
    private string _pushed = "";
    private int _sweeping;

    public PairingStore(ServerSecretsStore secrets, IActivityRecorder activity, TimeProvider time, ILogger<PairingStore> logger)
    {
        _secrets = secrets;
        _activity = activity;
        _time = time;
        _logger = logger;
        _sweeper = time.CreateTimer(_ => _ = SweepAsync(), null, SweepInterval, SweepInterval);
    }

    public event Func<IReadOnlyList<PairingRequestView>, Task>? Changed;

    /// <summary>Requests waiting for a person, newest first, and for a moment the ones that were just denied.</summary>
    public IReadOnlyList<PairingRequestView> Pending
    {
        get
        {
            var now = _time.GetUtcNow();
            return [.. _requests.Values
                .Where(r => IsVisible(r, now))
                .OrderByDescending(r => r.RequestedAt)
                .Select(r => new PairingRequestView(r.Id, r.ClientName, r.From, r.RequestedAt, r.ExpiresAt, r.State, r.State == Denied ? r.Message : null))];
        }
    }

    /// <summary>
    /// Step 1: a client commits to its nonce. Returns the request ID, the server's nonce and the poll
    /// token the client must present from now on, or null when the server takes no more requests.
    /// </summary>
    public PairingOpened? Open(string clientName, string commitment, IPAddress? from)
    {
        var now = _time.GetUtcNow();
        var key = AddressKey(from);
        lock (_admission)
        {
            while (_opens.Count > 0 && _opens.Peek() + OpenWindow <= now)
            {
                _opens.Dequeue();
            }

            if (_opens.Count >= OpensPerWindow
                || _requests.Values.Count(r => IsRevealedAndWaiting(r, now)) >= MaxOpen
                || _requests.Values.Count(r => r.AddressKey == key && IsLive(r, now)) >= MaxOpenPerAddress)
            {
                return null;
            }

            _opens.Enqueue(now);
            // The ID is the client's handle on its request; the poll token is the proof that it is the one asking. Both are unguessable.
            var request = new Request(
                Convert.ToHexString(RandomNumberGenerator.GetBytes(16)),
                clientName,
                Display(from),
                key,
                commitment,
                PairingCode.NewNonce(),
                Convert.ToHexString(RandomNumberGenerator.GetBytes(32)),
                now,
                now + Lifetime);
            _requests[request.Id] = request;
            return new PairingOpened(request.Id, request.ServerNonce, request.ExpiresAt, request.PollToken);
        }
    }

    /// <summary>Step 3: the client reveals its nonce. Only now is there a code, and the UI shows the request.</summary>
    public async Task<bool> RevealAsync(string id, string clientNonce)
    {
        if (!_requests.TryGetValue(id, out var request))
        {
            return false;
        }

        bool revealed;
        await request.Gate.WaitAsync();
        try
        {
            var now = _time.GetUtcNow();
            revealed = request.State == Waiting && request.Code is null && !IsStale(request, now) && request.ExpiresAt > now
                && PairingCode.Matches(request.Commitment, clientNonce);
            if (revealed)
            {
                request.Code = PairingCode.Compute(_secrets.Fingerprint, clientNonce, request.ServerNonce);
            }
        }
        finally
        {
            request.Gate.Release();
        }

        if (revealed)
        {
            await RaiseAsync();
        }

        return revealed;
    }

    /// <summary>
    /// The person typed a code. A match registers the client and keeps its credentials for it to
    /// collect; a miss counts against the request. Returns null when there is no such request.
    /// </summary>
    public async Task<bool?> ApproveAsync(string id, string typedCode, CancellationToken cancellationToken)
    {
        if (!_requests.TryGetValue(id, out var request))
        {
            return null;
        }

        bool? result;
        bool changed;
        await request.Gate.WaitAsync(cancellationToken);
        try
        {
            changed = Lapse(request, _time.GetUtcNow());
            if (request.State != Waiting || request.Code is null)
            {
                result = null;
            }
            else if (!PairingCode.IsCode(request.Code, typedCode))
            {
                result = false;
                if (++request.WrongCodes >= MaxWrongCodes)
                {
                    request.State = Denied;
                    request.Message = "The code was wrong three times.";
                    request.ShowUntil = _time.GetUtcNow() + DeniedVisibleFor;
                    changed = true;
                    await _activity.RecordAsync(Entity(request), ActivityReason.ClientAdded, ActivityOutcome.Failed, $"Pairing with {request.ClientName} was refused", request.Message);
                }
            }
            else
            {
                var (client, secret) = await _secrets.CreateClientAsync(request.ClientName, cancellationToken);
                request.ClientId = client.Id;
                request.ClientSecret = secret;
                request.State = Approved;
                changed = true;
                result = true;
                await _activity.RecordAsync(new EntityRef(client.Id, client.Name, "client"), ActivityReason.ClientAdded, ActivityOutcome.Succeeded, $"{client.Name} paired from {request.From}");
            }
        }
        finally
        {
            request.Gate.Release();
        }

        if (changed)
        {
            await RaiseAsync();
        }

        return result;
    }

    public async Task<bool> DenyAsync(string id)
    {
        if (!_requests.TryGetValue(id, out var request))
        {
            return false;
        }

        bool denied;
        await request.Gate.WaitAsync();
        try
        {
            Lapse(request, _time.GetUtcNow());
            denied = request.State == Waiting && request.Code is not null;
            if (denied)
            {
                request.State = Denied;
                await _activity.RecordAsync(Entity(request), ActivityReason.ClientAdded, ActivityOutcome.Failed, $"Pairing with {request.ClientName} was refused");
            }
        }
        finally
        {
            request.Gate.Release();
        }

        if (denied)
        {
            await RaiseAsync();
        }

        return denied;
    }

    /// <summary>The client gave up. Only the client holding the poll token can, and only for a request still waiting.</summary>
    public async Task<bool> WithdrawAsync(string id, string? pollToken)
    {
        if (!_requests.TryGetValue(id, out var request) || !HasToken(request, pollToken))
        {
            return false;
        }

        bool withdrawn;
        bool wasVisible;
        await request.Gate.WaitAsync();
        try
        {
            Lapse(request, _time.GetUtcNow());
            withdrawn = request.State == Waiting && _requests.TryRemove(id, out _);
            wasVisible = withdrawn && request.Code is not null;
        }
        finally
        {
            request.Gate.Release();
        }

        if (wasVisible)
        {
            await RaiseAsync();
        }

        return withdrawn;
    }

    /// <summary>The client polls. Credentials are handed over once, then the request is gone. Null for an unknown request or a wrong token.</summary>
    public async Task<PairingPoll?> PollAsync(string id, string? pollToken)
    {
        if (!_requests.TryGetValue(id, out var request) || !HasToken(request, pollToken))
        {
            return null;
        }

        PairingPoll poll;
        bool changed;
        await request.Gate.WaitAsync();
        try
        {
            changed = Lapse(request, _time.GetUtcNow());
            if (request.State == Approved)
            {
                _requests.TryRemove(id, out _);
                poll = new PairingPoll(Approved, request.ClientId, request.ClientSecret);
            }
            else
            {
                poll = new PairingPoll(request.State);
            }
        }
        finally
        {
            request.Gate.Release();
        }

        if (changed)
        {
            await RaiseAsync();
        }

        return poll;
    }

    public void Dispose() => _sweeper.Dispose();

    /// <summary>The address a limit counts: one IPv4 address, or one IPv6 /64, which is what a single network hands a host.</summary>
    internal static string AddressKey(IPAddress? address)
    {
        if (address is null)
        {
            return "unknown";
        }

        if (address.IsIPv4MappedToIPv6)
        {
            address = address.MapToIPv4();
        }

        return address.AddressFamily == AddressFamily.InterNetworkV6
            ? $"{Convert.ToHexString(address.GetAddressBytes().AsSpan(0, 8))}/64"
            : address.ToString();
    }

    private static string Display(IPAddress? address) =>
        address is null ? "unknown" : (address.IsIPv4MappedToIPv6 ? address.MapToIPv4() : address).ToString();

    private static EntityRef Entity(Request request) => new(request.Id, request.ClientName, "client");

    private static bool HasToken(Request request, string? presented) =>
        !string.IsNullOrEmpty(presented)
        && CryptographicOperations.FixedTimeEquals(Encoding.ASCII.GetBytes(request.PollToken), Encoding.ASCII.GetBytes(presented));

    /// <summary>A request whose client never revealed its nonce in time. It is as good as gone.</summary>
    private static bool IsStale(Request request, DateTimeOffset now) =>
        request.State == Waiting && request.Code is null && request.RequestedAt + UnrevealedLifetime <= now;

    private static bool IsLive(Request request, DateTimeOffset now) =>
        request.State == Waiting && request.ExpiresAt > now && !IsStale(request, now);

    private static bool IsRevealedAndWaiting(Request request, DateTimeOffset now) =>
        request.State == Waiting && request.Code is not null && request.ExpiresAt > now;

    private static bool IsVisible(Request request, DateTimeOffset now) =>
        IsRevealedAndWaiting(request, now) || (request.State == Denied && request.ShowUntil > now);

    /// <summary>Moves a waiting request past its lifetime to expired, with its gate held. True when the UI list changes with it.</summary>
    private static bool Lapse(Request request, DateTimeOffset now)
    {
        if (request.State != Waiting || request.ExpiresAt > now)
        {
            return false;
        }

        request.State = Expired;
        return request.Code is not null;
    }

    /// <summary>Expires requests, drops dead ones and pushes the list when what the UI shows changed, even if nobody called in.</summary>
    private async Task SweepAsync()
    {
        if (Interlocked.Exchange(ref _sweeping, 1) == 1)
        {
            return;
        }

        try
        {
            var now = _time.GetUtcNow();
            foreach (var request in _requests.Values)
            {
                await request.Gate.WaitAsync();
                try
                {
                    Lapse(request, now);
                    if (IsStale(request, now) || request.ExpiresAt + Lifetime <= now)
                    {
                        _requests.TryRemove(request.Id, out _);
                    }
                }
                finally
                {
                    request.Gate.Release();
                }
            }

            if (Signature(Pending) != Volatile.Read(ref _pushed))
            {
                await RaiseAsync();
            }
        }
        catch (Exception ex)
        {
            LogSweepFailed(ex);
        }
        finally
        {
            Volatile.Write(ref _sweeping, 0);
        }
    }

    private static string Signature(IReadOnlyList<PairingRequestView> list) => string.Join('|', list.Select(v => $"{v.Id}:{v.State}"));

    private Task RaiseAsync()
    {
        var list = Pending;
        Volatile.Write(ref _pushed, Signature(list));
        return Changed.RaiseAsync(list, _logger);
    }

    [LoggerMessage(Level = LogLevel.Error, Message = "Sweeping pairing requests failed")]
    private partial void LogSweepFailed(Exception exception);

    private sealed class Request(string id, string clientName, string from, string addressKey, string commitment, string serverNonce, string pollToken, DateTimeOffset requestedAt, DateTimeOffset expiresAt)
    {
        public string Id { get; } = id;
        public string ClientName { get; } = clientName;
        public string From { get; } = from;
        public string AddressKey { get; } = addressKey;
        public string Commitment { get; } = commitment;
        public string ServerNonce { get; } = serverNonce;
        public string PollToken { get; } = pollToken;
        public DateTimeOffset RequestedAt { get; } = requestedAt;
        public DateTimeOffset ExpiresAt { get; } = expiresAt;

        /// <summary>Held while a state change on this request runs.</summary>
        public SemaphoreSlim Gate { get; } = new(1, 1);

        public string? Code { get; set; }
        public string State { get; set; } = Waiting;
        public int WrongCodes { get; set; }
        public string? Message { get; set; }
        public DateTimeOffset? ShowUntil { get; set; }
        public string? ClientId { get; set; }
        public string? ClientSecret { get; set; }
    }
}
