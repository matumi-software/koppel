using System.Net;
using Koppel.Core.Configuration;
using Skakel.Client;

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

/// <summary>One pairing request on a server, as a run sees it: Skakel's pairing client, or a fake in tests.</summary>
internal interface IPairingRequest : IAsyncDisposable
{
    /// <summary>The six-digit code the person types on the server.</summary>
    string Code { get; }

    DateTimeOffset ExpiresAt { get; }

    /// <summary>Waits for the person to type the code on the server, and returns the credentials.</summary>
    Task<SkakelCredentials> WaitAsync(CancellationToken cancellationToken);
}

/// <summary>
/// Client role: pairs with a server instead of having its credentials typed in. Skakel opens the request
/// over HTTPS pinned to the server's announced fingerprint and works out the code (see Skakel's
/// <c>docs/protocol.md</c>); this shows the code and waits for the person on the server to type it. On
/// approval the credentials go straight into <c>connection.json</c>, and the usual <c>ConnectionChanged</c>
/// confirms it, unless the client is already connected to another server: then they wait for
/// <see cref="ConfirmAsync"/>.
/// </summary>
internal sealed partial class PairingClient : IDisposable
{
    /// <summary>Generous: a server on slow hardware can take many seconds over its first request after a start.</summary>
    private static readonly TimeSpan RequestTimeout = TimeSpan.FromSeconds(60);

    private readonly ConnectionStore _connection;
    private readonly ILogger<PairingClient> _logger;
    private readonly Lock _lock = new();
    private Run? _run;
    private PairingStatus _status = PairingStatus.Idle;

    public PairingClient(ConnectionStore connection, TimeProvider time, ILoggerFactory loggers)
    {
        _connection = connection;
        _logger = loggers.CreateLogger<PairingClient>();
        var options = new SkakelClientOptions { LoggerFactory = loggers, TimeProvider = time, RequestTimeout = RequestTimeout };
        Open = async (server, fingerprint, cancellationToken) =>
            new SkakelRequest(await SkakelPairingClient.OpenAsync(server, fingerprint, Environment.MachineName, options, cancellationToken));
        _connection.Changed += OnConnectionChangedAsync;
    }

    public event Func<PairingStatus, Task>? Changed;

    public PairingStatus Current => _status;

    /// <summary>Opens a request on the server at the address, pinned to the fingerprint. Skakel's unless a test brings a fake.</summary>
    internal Func<Uri, string, CancellationToken, Task<IPairingRequest>> Open { get; init; }

    /// <summary>Starts pairing in the background, replacing one already under way. The task ends with the run.</summary>
    public Task Begin(string serverUrl, string fingerprint, string? serverName)
    {
        var run = new Run(serverUrl, fingerprint, serverName ?? new Uri(serverUrl).Host);
        lock (_lock)
        {
            _run?.Cancel();
            _run = run;
        }

        return Task.Run(
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
        try
        {
            // Disposing a request that is still waiting withdraws it, so a cancelled one does not sit on the server's Clients page.
            await using var request = await Open(new Uri(run.ServerUrl), run.Fingerprint, cancellationToken);
            status = status with { State = "waiting", Code = request.Code, ExpiresAt = request.ExpiresAt };
            await SetAsync(run, status);
            LogWaiting(run.ServerName);

            var credentials = await request.WaitAsync(cancellationToken);
            await ApprovedAsync(run, status, credentials);
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            // Cancelled, or replaced by another pairing; the cancel or the newer run has the status.
        }
        catch (SkakelException ex)
        {
            LogFailed(run.ServerName, ex.Failure.Code);
            await SetAsync(run, Failed(run, status, ex));
        }
        catch (Exception ex)
        {
            LogFailed(run.ServerName, ex.Message);
            await SetAsync(run, status with { State = "failed", Code = null, Message = $"Could not pair with {run.ServerName}: {ex.Message}" });
        }
    }

    /// <summary>Skakel's failure as the status the UI shows.</summary>
    private static PairingStatus Failed(Run run, PairingStatus status, SkakelException ex) => ex.Failure.Kind switch
    {
        SkakelFailureKind.PairingDenied => status with { State = "denied", Code = null, Message = $"{run.ServerName} refused the request." },
        SkakelFailureKind.PairingExpired => status with { State = "expired", Code = null, Message = ex.Failure.Message },
        SkakelFailureKind.TooManyRequests => status with { State = "failed", Code = null, Message = $"{run.ServerName} is taking no more pairing requests for now. Try again in a few minutes." },

        // A machine that is a client only answers 404 to everything a pairing client asks.
        SkakelFailureKind.UnexpectedAnswer when ex.InnerException is HttpRequestException { StatusCode: HttpStatusCode.NotFound } =>
            status with { State = "failed", Code = null, Message = $"{run.ServerName} does not take pairing requests. It may be set up as a client only." },
        _ => status with { State = "failed", Code = null, Message = $"Could not pair with {run.ServerName}: {ex.Failure.Message}" },
    };

    /// <summary>
    /// The server approved. The credentials must be whole, and they become the connection as they are:
    /// nothing of the stored secret is carried over. Saved at once for a client with no connection, or
    /// one for the same server; otherwise held until the person confirms.
    /// </summary>
    private async Task ApprovedAsync(Run run, PairingStatus status, SkakelCredentials credentials)
    {
        if (string.IsNullOrWhiteSpace(credentials.ClientId) || string.IsNullOrEmpty(credentials.ClientSecret))
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

        var update = new ConnectionUpdate(run.ServerUrl, run.Fingerprint, credentials.ClientId, credentials.ClientSecret);
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
    private sealed class Run(string serverUrl, string fingerprint, string serverName) : IDisposable
    {
        private readonly CancellationTokenSource _cts = new();

        public string ServerUrl { get; } = serverUrl;
        public string Fingerprint { get; } = fingerprint;
        public string ServerName { get; } = serverName;
        public CancellationToken Token => _cts.Token;

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

    /// <summary>Skakel's pairing client as a run sees it.</summary>
    private sealed class SkakelRequest(SkakelPairingClient pairing) : IPairingRequest
    {
        public string Code => pairing.Code;

        public DateTimeOffset ExpiresAt => pairing.ExpiresAt;

        public Task<SkakelCredentials> WaitAsync(CancellationToken cancellationToken) => pairing.WaitAsync(cancellationToken);

        public ValueTask DisposeAsync() => pairing.DisposeAsync();
    }
}
