using System.Collections.Concurrent;
using System.Security.Cryptography;
using Koppel.Core.Activity;
using Koppel.Core.Security;

namespace Koppel.Server;

/// <summary>A pairing request as the server's UI sees it. The code is not in it: the person types it.</summary>
public sealed record PairingRequestView(string Id, string ClientName, string From, DateTimeOffset RequestedAt, DateTimeOffset ExpiresAt);

/// <summary>What a client learns when it polls its request.</summary>
/// <param name="State"><c>waiting</c>, <c>approved</c> (with the credentials, once), <c>denied</c> or <c>expired</c>.</param>
internal sealed record PairingPoll(string State, string? ClientId = null, string? ClientSecret = null);

/// <summary>
/// Server role: pairing requests from clients, kept in memory for a few minutes. A request becomes
/// visible in the UI only after the client revealed its nonce, so the code exists on both ends. The
/// person approves it by typing the code the client shows; three wrong codes drop the request.
/// </summary>
internal sealed class PairingStore(ServerSecretsStore secrets, IActivityRecorder activity, TimeProvider time)
{
    public static readonly TimeSpan Lifetime = TimeSpan.FromMinutes(5);
    private const int MaxOpen = 10;
    private const int MaxOpenPerAddress = 3;
    private const int MaxWrongCodes = 3;

    private readonly ConcurrentDictionary<string, Request> _requests = new();

    public event Func<IReadOnlyList<PairingRequestView>, Task>? Changed;

    /// <summary>Requests waiting for a person, newest first.</summary>
    public IReadOnlyList<PairingRequestView> Pending
    {
        get
        {
            Expire();
            return [.. _requests.Values
                .Where(r => r.State == "waiting" && r.Code is not null)
                .OrderByDescending(r => r.RequestedAt)
                .Select(r => new PairingRequestView(r.Id, r.ClientName, r.From, r.RequestedAt, r.ExpiresAt))];
        }
    }

    /// <summary>Step 1: a client commits to its nonce. Returns the request ID and the server's nonce, or null when too many are open.</summary>
    public (string Id, string ServerNonce, DateTimeOffset ExpiresAt)? Open(string clientName, string commitment, string from)
    {
        Expire();
        if (_requests.Count >= MaxOpen || _requests.Values.Count(r => r.From == from) >= MaxOpenPerAddress)
        {
            return null;
        }

        var now = time.GetUtcNow();
        // The ID is the client's only handle on the request and its credentials, so it is unguessable.
        var request = new Request(Convert.ToHexString(RandomNumberGenerator.GetBytes(16)), clientName, from, commitment, PairingCode.NewNonce(), now, now + Lifetime);
        _requests[request.Id] = request;
        return (request.Id, request.ServerNonce, request.ExpiresAt);
    }

    /// <summary>Step 3: the client reveals its nonce. Only now is there a code, and the UI shows the request.</summary>
    public async Task<bool> RevealAsync(string id, string clientNonce)
    {
        if (!_requests.TryGetValue(id, out var request) || request.Code is not null || !PairingCode.Matches(request.Commitment, clientNonce))
        {
            return false;
        }

        request.Code = PairingCode.Compute(secrets.Fingerprint, clientNonce, request.ServerNonce);
        await RaiseAsync();
        return true;
    }

    /// <summary>
    /// The person typed a code. A match registers the client and keeps its credentials for it to
    /// collect; a miss counts against the request. Returns null when there is no such request.
    /// </summary>
    public async Task<bool?> ApproveAsync(string id, string typedCode, CancellationToken cancellationToken)
    {
        Expire();
        if (!_requests.TryGetValue(id, out var request) || request.State != "waiting" || request.Code is null)
        {
            return null;
        }

        if (!PairingCode.IsCode(request.Code, typedCode))
        {
            if (++request.WrongCodes >= MaxWrongCodes)
            {
                request.State = "denied";
                await activity.RecordAsync(Entity(request), ActivityReason.ClientAdded, ActivityOutcome.Failed, $"Pairing with {request.ClientName} was refused", "The code was wrong three times.");
                await RaiseAsync();
            }

            return false;
        }

        var (client, secret) = await secrets.CreateClientAsync(request.ClientName, cancellationToken);
        request.ClientId = client.Id;
        request.ClientSecret = secret;
        request.State = "approved";
        await activity.RecordAsync(new EntityRef(client.Id, client.Name, "client"), ActivityReason.ClientAdded, ActivityOutcome.Succeeded, $"{client.Name} paired from {request.From}");
        await RaiseAsync();
        return true;
    }

    public async Task<bool> DenyAsync(string id)
    {
        if (!_requests.TryGetValue(id, out var request) || request.State != "waiting")
        {
            return false;
        }

        request.State = "denied";
        await activity.RecordAsync(Entity(request), ActivityReason.ClientAdded, ActivityOutcome.Failed, $"Pairing with {request.ClientName} was refused");
        await RaiseAsync();
        return true;
    }

    /// <summary>The client polls. Credentials are handed over once, then the request is gone.</summary>
    public PairingPoll? Poll(string id)
    {
        Expire();
        if (!_requests.TryGetValue(id, out var request))
        {
            return null;
        }

        if (request.State == "waiting")
        {
            return new PairingPoll("waiting");
        }

        _requests.TryRemove(id, out _);
        return new PairingPoll(request.State, request.ClientId, request.ClientSecret);
    }

    private static EntityRef Entity(Request request) => new(request.Id, request.ClientName, "client");

    /// <summary>Drops requests past their lifetime. Expired ones answer "expired" once more if polled soon after.</summary>
    private void Expire()
    {
        var now = time.GetUtcNow();
        foreach (var request in _requests.Values)
        {
            if (request.State == "waiting" && request.ExpiresAt <= now)
            {
                request.State = "expired";
                _ = RaiseAsync();
            }
            else if (request.ExpiresAt + Lifetime <= now)
            {
                _requests.TryRemove(request.Id, out _);
            }
        }
    }

    private Task RaiseAsync() => Changed?.Invoke(Pending) ?? Task.CompletedTask;

    private sealed class Request(string id, string clientName, string from, string commitment, string serverNonce, DateTimeOffset requestedAt, DateTimeOffset expiresAt)
    {
        public string Id { get; } = id;
        public string ClientName { get; } = clientName;
        public string From { get; } = from;
        public string Commitment { get; } = commitment;
        public string ServerNonce { get; } = serverNonce;
        public DateTimeOffset RequestedAt { get; } = requestedAt;
        public DateTimeOffset ExpiresAt { get; } = expiresAt;
        public string? Code { get; set; }
        public string State { get; set; } = "waiting";
        public int WrongCodes { get; set; }
        public string? ClientId { get; set; }
        public string? ClientSecret { get; set; }
    }
}
