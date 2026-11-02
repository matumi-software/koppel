using System.Net;
using System.Net.Http.Json;
using Koppel.Core.Configuration;
using Koppel.Core.Security;
using Koppel.Server;

namespace Koppel.Client;

/// <summary>Where this client's pairing stands, for the UI.</summary>
/// <param name="State">
/// <c>idle</c>, <c>requesting</c>, <c>waiting</c> (show <see cref="Code"/>), <c>confirm</c> (approved, but saving replaces
/// the connection to another server, so the person decides; show <see cref="ServerName"/> and <see cref="ServerFingerprint"/>),
/// <c>paired</c>, <c>denied</c>, <c>expired</c> or <c>failed</c>.
/// </param>
public sealed record PairingStatus(string State, string? ServerName = null, string? ServerUrl = null, string? Code = null, string? Message = null, DateTimeOffset? ExpiresAt = null, string? ServerFingerprint = null)
{
    public static PairingStatus Idle { get; } = new("idle");
}

internal sealed record PairStartRequest(string? ServerUrl, string? ServerFingerprint, string? ServerName);

/// <summary>
/// Client role: pairs with a server instead of having its credentials typed in. Opens a request over
/// HTTPS pinned to the server's announced fingerprint, shows the code, and waits for the person on the
/// server to type it. On approval the credentials go straight into <c>connection.json</c>, and the
/// usual <c>ConnectionChanged</c> confirms it, unless the client is already connected to another
/// server: then they wait for <see cref="ConfirmAsync"/>. See <see cref="PairingCode"/> for the protocol.
/// </summary>
internal sealed partial class PairingClient : IDisposable
{
    private static readonly TimeSpan PollInterval = TimeSpan.FromSeconds(2);

    /// <summary>Generous: a server on slow hardware can take many seconds over its first request after a start.</summary>
    private static readonly TimeSpan RequestTimeout = TimeSpan.FromSeconds(60);

    private readonly ConnectionStore _connection;
    private readonly TimeProvider _time;
    private readonly ILogger<PairingClient> _logger;
    private readonly Lock _lock = new();
    private Run? _run;
    private PairingStatus _status = PairingStatus.Idle;

    public PairingClient(ConnectionStore connection, TimeProvider time, ILogger<PairingClient> logger)
    {
        _connection = connection;
        _time = time;
        _logger = logger;
        _connection.Changed += OnConnectionChangedAsync;
    }

    public event Func<PairingStatus, Task>? Changed;

    public PairingStatus Current => _status;

    /// <summary>How a run reaches the server. The default pins the server's fingerprint; tests bring a fake.</summary>
    internal Func<string, HttpMessageHandler> HandlerFactory { get; init; } = ServerCertificate.CreatePinnedHandler;

    /// <summary>Starts pairing in the background, replacing one already under way.</summary>
    public void Begin(string serverUrl, string fingerprint, string? serverName)
    {
        var run = new Run(serverUrl, fingerprint, serverName ?? new Uri(serverUrl).Host, PairingStore.Lifetime + TimeSpan.FromSeconds(30), _time);
        lock (_lock)
        {
            _run?.Cancel();
            _run = run;
        }

        run.Task = Task.Run(
            async () =>
            {
                try
                {
                    await RunAsync(run);
                }
                finally
                {
                    run.Dispose();
                }
            },
            CancellationToken.None);
    }

    /// <summary>Ends the run, withdraws its request and drops any credentials it holds.</summary>
    public Task CancelAsync()
    {
        lock (_lock)
        {
            _run?.Cancel();
            _run = null;
            _status = PairingStatus.Idle;
        }

        return Changed.RaiseAsync(PairingStatus.Idle, _logger);
    }

    /// <summary>
    /// Saves the credentials the server approved, after the person saw which server they are for and
    /// agreed to replace the current connection. False when nothing waits for confirmation.
    /// </summary>
    public async Task<bool> ConfirmAsync(CancellationToken cancellationToken)
    {
        Run run;
        ConnectionUpdate update;
        lock (_lock)
        {
            if (_run is not { Approved: { }, Confirming: false } current || _status.State != "confirm")
            {
                return false;
            }

            run = current;
            update = run.Approved!;
            run.Confirming = true;
        }

        try
        {
            await _connection.UpdateAsync(update.ApplyTo, cancellationToken);
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        {
            LogFailed(run.ServerName, ex.Message);
            await SetAsync(run, new PairingStatus("failed", run.ServerName, run.ServerUrl, Message: $"Could not save the connection to {run.ServerName}: {ex.Message}"));
            return true;
        }

        LogPaired(run.ServerName);
        await SetAsync(run, Paired(run));
        return true;
    }

    public void Dispose() => _connection.Changed -= OnConnectionChangedAsync;

    private async Task RunAsync(Run run)
    {
        var status = new PairingStatus("requesting", run.ServerName, run.ServerUrl, ServerFingerprint: run.Fingerprint);
        await SetAsync(run, status);
        var cancellationToken = run.Token;
        using var http = new HttpClient(HandlerFactory(run.Fingerprint)) { BaseAddress = new Uri(run.ServerUrl), Timeout = RequestTimeout };
        string? requestId = null;
        string? pollToken = null;
        try
        {
            var nonce = PairingCode.NewNonce();
            using var open = await http.PostAsJsonAsync("/api/pair", new PairOpenRequest(Environment.MachineName, PairingCode.Commit(nonce)), KoppelJson.Files, cancellationToken);
            if (open.StatusCode == HttpStatusCode.TooManyRequests)
            {
                await SetAsync(run, status with { State = "failed", Message = $"{run.ServerName} has too many pairing requests open. Try again in a few minutes." });
                return;
            }

            if (open.StatusCode == HttpStatusCode.NotFound)
            {
                await SetAsync(run, status with { State = "failed", Message = $"{run.ServerName} does not take pairing requests. It may be set up as a client only." });
                return;
            }

            open.EnsureSuccessStatusCode();
            var opened = await open.Content.ReadFromJsonAsync<PairOpened>(KoppelJson.Files, cancellationToken);
            if (opened is not { RequestId.Length: > 0, ServerNonce.Length: > 0, PollToken.Length: > 0 })
            {
                throw new InvalidDataException("The server's reply was not what Koppel expects.");
            }

            requestId = opened.RequestId;
            pollToken = opened.PollToken;
            using (var revealed = await http.PostAsJsonAsync($"/api/pair/{opened.RequestId}/reveal", new PairRevealRequest(nonce), KoppelJson.Files, cancellationToken))
            {
                revealed.EnsureSuccessStatusCode();
            }

            status = status with { State = "waiting", Code = PairingCode.Compute(run.Fingerprint, nonce, opened.ServerNonce), ExpiresAt = opened.ExpiresAt };
            await SetAsync(run, status);
            LogWaiting(run.ServerName);

            while (true)
            {
                await Task.Delay(PollInterval, _time, cancellationToken);
                using var request = new HttpRequestMessage(HttpMethod.Get, $"/api/pair/{opened.RequestId}");
                request.Headers.Add(PairingEndpoints.TokenHeader, opened.PollToken);
                using var response = await http.SendAsync(request, cancellationToken);
                var poll = response.StatusCode == HttpStatusCode.NotFound
                    ? new PairingPoll("expired")
                    : await response.EnsureSuccessStatusCode().Content.ReadFromJsonAsync<PairingPoll>(KoppelJson.Files, cancellationToken);
                switch (poll?.State)
                {
                    case "approved":
                        await ApprovedAsync(run, status, poll);
                        return;
                    case "denied":
                        await SetAsync(run, status with { State = "denied", Code = null, Message = $"{run.ServerName} refused the request." });
                        return;
                    case "expired":
                        await SetAsync(run, status with { State = "expired", Code = null, Message = "Nobody typed the code in time. Start again." });
                        return;
                }
            }
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            // Cancelled, replaced by another pairing, or out of time. Withdraw the request, so it does
            // not sit on the server's Clients page until it expires.
            await WithdrawAsync(http, requestId, pollToken);

            // Only a run that is still the current one, and so ran out of time, may say so.
            await SetAsync(run, status with { State = "expired", Code = null, Message = "Nobody typed the code in time. Start again." });
        }
        catch (TaskCanceledException)
        {
            LogFailed(run.ServerName, "timed out");
            await SetAsync(run, status with { State = "failed", Code = null, Message = $"{run.ServerName} did not answer in time." });
        }
        catch (Exception ex)
        {
            LogFailed(run.ServerName, ex.Message);
            await SetAsync(run, status with { State = "failed", Code = null, Message = $"Could not pair with {run.ServerName}: {ex.Message}" });
        }
    }

    /// <summary>
    /// The server approved. The credentials must be whole, and they become the connection as they are:
    /// nothing of the stored secret is carried over. Saved at once for a client with no connection, or
    /// one for the same server; otherwise held until the person confirms.
    /// </summary>
    private async Task ApprovedAsync(Run run, PairingStatus status, PairingPoll poll)
    {
        if (string.IsNullOrWhiteSpace(poll.ClientId) || string.IsNullOrEmpty(poll.ClientSecret))
        {
            LogFailed(run.ServerName, "approved without credentials");
            await SetAsync(run, status with { State = "failed", Code = null, Message = $"{run.ServerName} approved the request but did not send credentials. Start again." });
            return;
        }

        // A run that was cancelled or replaced while its poll was in flight must not save anything.
        lock (_lock)
        {
            if (!ReferenceEquals(_run, run))
            {
                return;
            }
        }

        var update = new ConnectionUpdate(run.ServerUrl, run.Fingerprint, poll.ClientId, poll.ClientSecret);
        var existing = _connection.Current;
        if ((existing.ServerUrl is not null || existing.ClientId is not null) && !update.KeepsServer(existing))
        {
            lock (_lock)
            {
                run.Approved = update;
            }

            LogHeld(run.ServerName);
            await SetAsync(run, status with
            {
                State = "confirm",
                Code = null,
                Message = $"{run.ServerName} approved this machine. Saving replaces your connection to {existing.ServerUrl}.",
            });
            return;
        }

        try
        {
            await _connection.UpdateAsync(update.ApplyTo, CancellationToken.None);
        }
        catch (Exception ex)
        {
            LogFailed(run.ServerName, ex.Message);
            await SetAsync(run, status with { State = "failed", Code = null, Message = $"Could not save the connection to {run.ServerName}: {ex.Message}" });
            return;
        }

        lock (_lock)
        {
            run.Approved = update;
        }

        LogPaired(run.ServerName);
        await SetAsync(run, Paired(run));
    }

    private static PairingStatus Paired(Run run) => new("paired", run.ServerName, run.ServerUrl, ServerFingerprint: run.Fingerprint);

    private static async Task WithdrawAsync(HttpClient http, string? requestId, string? pollToken)
    {
        if (requestId is null || pollToken is null)
        {
            return;
        }

        try
        {
            using var request = new HttpRequestMessage(HttpMethod.Delete, $"/api/pair/{requestId}");
            request.Headers.Add(PairingEndpoints.TokenHeader, pollToken);
            using var withdrawn = await http.SendAsync(request, CancellationToken.None);
        }
        catch (Exception ex) when (ex is HttpRequestException or TaskCanceledException)
        {
            // The server expires it on its own.
        }
    }

    /// <summary>Sets the status for a run, unless another run or a cancel has taken over since.</summary>
    private Task SetAsync(Run run, PairingStatus status)
    {
        lock (_lock)
        {
            if (!ReferenceEquals(_run, run))
            {
                return Task.CompletedTask;
            }

            _status = status;
        }

        return Changed.RaiseAsync(status, _logger);
    }

    /// <summary>"Paired" only holds while the stored connection is the one it made. Once it changed, the claim goes.</summary>
    private Task OnConnectionChangedAsync(ConnectionView view)
    {
        lock (_lock)
        {
            if (_status.State != "paired" || _run is not { Approved: { } made } || MatchesView(made, view))
            {
                return Task.CompletedTask;
            }

            _run = null;
            _status = PairingStatus.Idle;
        }

        return Changed.RaiseAsync(PairingStatus.Idle, _logger);
    }

    private static bool MatchesView(ConnectionUpdate made, ConnectionView view) =>
        view.ClientId == made.ClientId?.Trim()
        && string.Equals(view.ServerUrl, made.ServerUrl, StringComparison.OrdinalIgnoreCase)
        && ConnectionUpdate.SameFingerprint(view.ServerFingerprint, made.ServerFingerprint);

    [LoggerMessage(Level = LogLevel.Information, Message = "Waiting for the code to be typed on {Server}")]
    private partial void LogWaiting(string server);

    [LoggerMessage(Level = LogLevel.Information, Message = "Paired with {Server}")]
    private partial void LogPaired(string server);

    [LoggerMessage(Level = LogLevel.Information, Message = "{Server} approved; waiting for the person to confirm replacing the connection")]
    private partial void LogHeld(string server);

    [LoggerMessage(Level = LogLevel.Warning, Message = "Pairing with {Server} failed: {Reason}")]
    private partial void LogFailed(string server, string reason);

    /// <summary>One pairing attempt. Its identity decides who may set the status; its credentials wait here for a confirmation.</summary>
    private sealed class Run(string serverUrl, string fingerprint, string serverName, TimeSpan lifetime, TimeProvider time) : IDisposable
    {
        private readonly CancellationTokenSource _cts = new(lifetime, time);

        public string ServerUrl { get; } = serverUrl;
        public string Fingerprint { get; } = fingerprint;
        public string ServerName { get; } = serverName;
        public CancellationToken Token => _cts.Token;
        public Task? Task { get; set; }

        /// <summary>What the server approved: held for a confirmation, or what was saved once paired.</summary>
        public ConnectionUpdate? Approved { get; set; }

        /// <summary>Set once a confirmation is saving, so a second one cannot save again.</summary>
        public bool Confirming { get; set; }

        public void Cancel()
        {
            try
            {
                _cts.Cancel();
            }
            catch (ObjectDisposedException)
            {
                // The run already ended.
            }
        }

        public void Dispose() => _cts.Dispose();
    }
}
