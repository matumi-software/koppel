using System.Net;
using System.Net.Http.Json;
using Koppel.Core.Security;
using Koppel.Server;

namespace Koppel.Client;

/// <summary>Where this client's pairing stands, for the UI.</summary>
/// <param name="State"><c>idle</c>, <c>requesting</c>, <c>waiting</c> (show <see cref="Code"/>), <c>paired</c>, <c>denied</c>, <c>expired</c> or <c>failed</c>.</param>
public sealed record PairingStatus(string State, string? ServerName = null, string? ServerUrl = null, string? Code = null, string? Message = null, DateTimeOffset? ExpiresAt = null)
{
    public static PairingStatus Idle { get; } = new("idle");
}

internal sealed record PairStartRequest(string? ServerUrl, string? ServerFingerprint, string? ServerName);

/// <summary>
/// Client role: pairs with a server instead of having its credentials typed in. Opens a request over
/// HTTPS pinned to the server's announced fingerprint, shows the code, and waits for the person on the
/// server to type it. On approval the credentials go straight into <c>connection.json</c>, and the
/// usual <c>ConnectionChanged</c> confirms it. See <see cref="PairingCode"/> for the protocol.
/// </summary>
internal sealed partial class PairingClient(ConnectionStore connection, ILogger<PairingClient> logger)
{
    private static readonly TimeSpan PollInterval = TimeSpan.FromSeconds(2);

    /// <summary>Generous: a server on slow hardware can take many seconds over its first request after a start.</summary>
    private static readonly TimeSpan RequestTimeout = TimeSpan.FromSeconds(60);
    private readonly Lock _lock = new();
    private CancellationTokenSource? _run;
    private PairingStatus _status = PairingStatus.Idle;

    public event Func<PairingStatus, Task>? Changed;

    public PairingStatus Current => _status;

    /// <summary>Starts pairing in the background, replacing one already under way.</summary>
    public void Begin(string serverUrl, string fingerprint, string? serverName)
    {
        CancellationTokenSource run;
        lock (_lock)
        {
            _run?.Cancel();
            _run = run = new CancellationTokenSource(PairingStore.Lifetime + TimeSpan.FromSeconds(30));
        }

        _ = Task.Run(() => RunAsync(serverUrl, fingerprint, serverName ?? new Uri(serverUrl).Host, run.Token), CancellationToken.None);
    }

    public Task CancelAsync()
    {
        lock (_lock)
        {
            _run?.Cancel();
            _run = null;
        }

        return SetAsync(PairingStatus.Idle);
    }

    private async Task RunAsync(string serverUrl, string fingerprint, string serverName, CancellationToken cancellationToken)
    {
        var status = new PairingStatus("requesting", serverName, serverUrl);
        await SetAsync(status);
        using var http = new HttpClient(ServerCertificate.CreatePinnedHandler(fingerprint)) { BaseAddress = new Uri(serverUrl), Timeout = RequestTimeout };
        string? requestId = null;
        try
        {
            var nonce = PairingCode.NewNonce();
            using var open = await http.PostAsJsonAsync("/api/pair", new PairOpenRequest(Environment.MachineName, PairingCode.Commit(nonce)), KoppelJson.Files, cancellationToken);
            if (open.StatusCode == HttpStatusCode.TooManyRequests)
            {
                await SetAsync(status with { State = "failed", Message = $"{serverName} has too many pairing requests open. Try again in a few minutes." });
                return;
            }

            if (open.StatusCode == HttpStatusCode.NotFound)
            {
                await SetAsync(status with { State = "failed", Message = $"{serverName} does not take pairing requests. It may be set up as a client only." });
                return;
            }

            open.EnsureSuccessStatusCode();
            var opened = (await open.Content.ReadFromJsonAsync<PairOpened>(KoppelJson.Files, cancellationToken))!;
            requestId = opened.RequestId;
            using (var revealed = await http.PostAsJsonAsync($"/api/pair/{opened.RequestId}/reveal", new PairRevealRequest(nonce), KoppelJson.Files, cancellationToken))
            {
                revealed.EnsureSuccessStatusCode();
            }

            status = status with { State = "waiting", Code = PairingCode.Compute(fingerprint, nonce, opened.ServerNonce), ExpiresAt = opened.ExpiresAt };
            await SetAsync(status);
            LogWaiting(serverName);

            while (true)
            {
                await Task.Delay(PollInterval, cancellationToken);
                using var response = await http.GetAsync($"/api/pair/{opened.RequestId}", cancellationToken);
                var poll = response.StatusCode == HttpStatusCode.NotFound
                    ? new PairingPoll("expired")
                    : await response.EnsureSuccessStatusCode().Content.ReadFromJsonAsync<PairingPoll>(KoppelJson.Files, cancellationToken);
                switch (poll?.State)
                {
                    case "approved":
                        var update = new ConnectionUpdate(serverUrl, fingerprint, poll.ClientId, poll.ClientSecret);
                        await connection.UpdateAsync(update.ApplyTo, CancellationToken.None);
                        LogPaired(serverName);
                        await SetAsync(status with { State = "paired", Code = null });
                        return;
                    case "denied":
                        await SetAsync(status with { State = "denied", Code = null, Message = $"{serverName} refused the request." });
                        return;
                    case "expired":
                        await SetAsync(status with { State = "expired", Code = null, Message = "Nobody typed the code in time. Start again." });
                        return;
                }
            }
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            // Cancelled, replaced by another pairing, or out of time; CancelAsync sets the status.
            // Withdraw the request, so it does not sit on the server's Clients page until it expires.
            if (requestId is not null)
            {
                try
                {
                    using var withdrawn = await http.DeleteAsync($"/api/pair/{requestId}", CancellationToken.None);
                }
                catch (HttpRequestException)
                {
                    // The server expires it on its own.
                }
            }

            if (_status.State == "waiting" && _status.ServerUrl == serverUrl)
            {
                await SetAsync(status with { State = "expired", Code = null, Message = "Nobody typed the code in time. Start again." });
            }
        }
        catch (Exception ex) when (ex is HttpRequestException or TaskCanceledException or System.Text.Json.JsonException)
        {
            LogFailed(serverName, ex.Message);
            await SetAsync(status with { State = "failed", Code = null, Message = $"Could not pair with {serverName}: {ex.Message}" });
        }
    }

    private Task SetAsync(PairingStatus status)
    {
        _status = status;
        return Changed?.Invoke(status) ?? Task.CompletedTask;
    }

    [LoggerMessage(Level = LogLevel.Information, Message = "Waiting for the code to be typed on {Server}")]
    private partial void LogWaiting(string server);

    [LoggerMessage(Level = LogLevel.Information, Message = "Paired with {Server}")]
    private partial void LogPaired(string server);

    [LoggerMessage(Level = LogLevel.Warning, Message = "Pairing with {Server} failed: {Reason}")]
    private partial void LogFailed(string server, string reason);
}
