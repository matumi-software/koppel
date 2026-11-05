using Skakel.Client;

namespace Koppel.Client;

/// <summary>Tests the saved connection: Skakel's hello, over HTTPS pinned to the server's certificate, with a token.</summary>
internal sealed class RemoteServer(ConnectionStore connection, TimeProvider time, ILoggerFactory loggers)
{
    /// <summary>How long the test may take, the token request and the hello together.</summary>
    private static readonly TimeSpan Timeout = TimeSpan.FromSeconds(10);

    /// <summary>
    /// Gets a token and calls the server's hello endpoint. The answer is one of a fixed set of outcomes
    /// with fixed wording; what went wrong in detail is logged by Skakel and never sent to the caller, so
    /// the test cannot be used to probe addresses and ports.
    /// </summary>
    public async Task<ConnectionTestResult> TestAsync(CancellationToken cancellationToken)
    {
        var settings = connection.Current;
        if (!settings.IsComplete)
        {
            return ConnectionTestResult.Of(ConnectionTestOutcome.Incomplete);
        }

        var options = new SkakelClientOptions { LoggerFactory = loggers, TimeProvider = time, RequestTimeout = Timeout };
        await using var link = new SkakelLink(settings.GetAddress(), settings.GetPins(), settings.GetCredentials(), options);
        try
        {
            var hello = await link.HelloAsync(cancellationToken);
            return ConnectionTestResult.Of(ConnectionTestOutcome.Ok, $"Connected to {hello.Server} as {hello.ClientId}.");
        }
        catch (SkakelException ex)
        {
            return ConnectionTestResult.From(ex.Failure);
        }
    }
}

/// <summary>How a connection test ended. A fixed set: the UI shows the message, tests and scripts read the outcome.</summary>
internal enum ConnectionTestOutcome
{
    Ok,
    Incomplete,
    Unreachable,
    CertificateMismatch,
    RejectedCredentials,
    ClientUnknown,
    VersionMismatch,
    UnexpectedAnswer,
    TooManyRequests,
}

internal sealed record ConnectionTestResult(bool Ok, ConnectionTestOutcome Outcome, string Message)
{
    public static ConnectionTestResult Of(ConnectionTestOutcome outcome, string? ok = null) => new(outcome == ConnectionTestOutcome.Ok, outcome, outcome switch
    {
        ConnectionTestOutcome.Ok => ok ?? "Connected.",
        ConnectionTestOutcome.Incomplete => "Fill in the server address, fingerprint, client ID and secret first.",
        ConnectionTestOutcome.Unreachable => "Could not reach the server. Check the address and that the server is running.",
        ConnectionTestOutcome.CertificateMismatch => "The server's certificate does not match the fingerprint. Copy the fingerprint from the server's Clients section again.",
        ConnectionTestOutcome.RejectedCredentials => "The server refused the client ID or secret. Check them, or ask for a new client.",
        ConnectionTestOutcome.VersionMismatch => SkakelFailure.Of(SkakelFailureKind.VersionMismatch).Message,
        ConnectionTestOutcome.UnexpectedAnswer => "The server did not answer like a Koppel server. Check the address and the port.",
        ConnectionTestOutcome.TooManyRequests => SkakelFailure.Of(SkakelFailureKind.TooManyRequests).Message,
        _ => "The server accepted the token but no longer knows this client. Ask for a new client.",
    });

    /// <summary>
    /// Skakel's failure as one of the outcomes. A version mismatch keeps Skakel's wording, which names both
    /// sides' versions. Anything a test cannot run into counts as unreachable.
    /// </summary>
    public static ConnectionTestResult From(SkakelFailure failure) => failure.Kind switch
    {
        SkakelFailureKind.CertificateMismatch => Of(ConnectionTestOutcome.CertificateMismatch),
        SkakelFailureKind.RejectedCredentials => Of(ConnectionTestOutcome.RejectedCredentials),
        SkakelFailureKind.ClientUnknown or SkakelFailureKind.Revoked or SkakelFailureKind.Rotated => Of(ConnectionTestOutcome.ClientUnknown),
        SkakelFailureKind.VersionMismatch => new(false, ConnectionTestOutcome.VersionMismatch, failure.Message),
        SkakelFailureKind.UnexpectedAnswer => Of(ConnectionTestOutcome.UnexpectedAnswer),
        SkakelFailureKind.TooManyRequests => Of(ConnectionTestOutcome.TooManyRequests),
        _ => Of(ConnectionTestOutcome.Unreachable),
    };
}
