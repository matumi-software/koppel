using Koppel.Core.Activity;
using Koppel.Server;
using Koppel.Web;

namespace Koppel.Login;

internal enum Verdict
{
    Ok,
    Wrong,
    Locked,
    Busy,
    Invalid,
    NotSet,
    AlreadySet,
}

/// <param name="RetryAfter">With <see cref="Verdict.Locked"/>: how long until another try is allowed.</param>
/// <param name="Problem">With <see cref="Verdict.Invalid"/>: what is wrong with the new password.</param>
internal sealed record SignInResult(Verdict Verdict, TimeSpan? RetryAfter = null, string? Problem = null);

/// <summary>
/// What the sign-in endpoints decide: whether a password or setup code is right, how many wrong tries a
/// caller has had, and what is written to the log and the activity log when one is wrong.
/// </summary>
internal sealed partial class UiSignIn(UiAuthStore store, UiLoginThrottle throttle, UiPasswordGate gate, IActivityRecorder activity, ILogger<UiSignIn> logger)
{
    private static readonly SignInResult Busy = new(Verdict.Busy);

    public async Task<SignInResult> LoginAsync(string caller, string? password)
    {
        if (!store.HasPassword)
        {
            return new SignInResult(Verdict.NotSet);
        }

        // A slot first (answering at once when none is free), then the attempt is booked before the
        // password is looked at, so parallel guesses cannot all get through ahead of the lock.
        using var slot = gate.TryEnter();
        if (slot is null)
        {
            return Busy;
        }

        var key = $"login:{caller}";
        var attempt = throttle.Reserve(key);
        if (!attempt.Granted)
        {
            return new SignInResult(Verdict.Locked, attempt.Lock);
        }

        if (store.VerifyPassword(password))
        {
            throttle.Clear(key);
            return new SignInResult(Verdict.Ok);
        }

        await FailAsync(attempt, caller, "Sign-in");
        return new SignInResult(Verdict.Wrong);
    }

    /// <summary>
    /// Sets the first password. The code is short (about 40 bits), so each caller is limited; one
    /// caller's wrong guesses never lock out a correct code from another. The rate limiter and the
    /// shared hash slots hold back the rest.
    /// </summary>
    public async Task<SignInResult> SetupAsync(string caller, string? code, string? password, CancellationToken cancellationToken)
    {
        if (store.HasPassword)
        {
            return new SignInResult(Verdict.AlreadySet);
        }

        if (UiPassword.Validate(password) is { } problem)
        {
            return new SignInResult(Verdict.Invalid, Problem: problem);
        }

        using var slot = gate.TryEnter();
        if (slot is null)
        {
            return Busy;
        }

        var mine = $"setup:{caller}";
        var attempt = throttle.Reserve(mine);
        if (!attempt.Granted)
        {
            return new SignInResult(Verdict.Locked, attempt.Lock);
        }

        switch (await store.SetFirstPasswordAsync(code, password!, cancellationToken))
        {
            case SetupOutcome.Done:
                throttle.Clear(mine);
                return new SignInResult(Verdict.Ok);
            case SetupOutcome.AlreadySet:
                return new SignInResult(Verdict.AlreadySet);
            default:
                await FailAsync(attempt, caller, "Setup with the setup code");
                return new SignInResult(Verdict.Wrong);
        }
    }

    public async Task<SignInResult> ChangePasswordAsync(string caller, string? current, string? next, string session, CancellationToken cancellationToken)
    {
        using var slot = gate.TryEnter();
        if (slot is null)
        {
            return Busy;
        }

        var key = $"login:{caller}";
        var attempt = throttle.Reserve(key);
        if (!attempt.Granted)
        {
            return new SignInResult(Verdict.Locked, attempt.Lock);
        }

        if (!store.VerifyPassword(current))
        {
            await FailAsync(attempt, caller, "Password change");
            return new SignInResult(Verdict.Wrong);
        }

        throttle.Clear(key);
        if (UiPassword.Validate(next) is { } problem)
        {
            return new SignInResult(Verdict.Invalid, Problem: problem);
        }

        await store.ChangePasswordAsync(next!, session, cancellationToken);
        LogPasswordChanged(caller);
        await activity.RecordAsync(Machine, ActivityReason.PasswordChanged, ActivityOutcome.Succeeded, "The UI password was changed and other sessions were signed out");
        return new SignInResult(Verdict.Ok);
    }

    /// <summary>What a caller's limit counts: one IPv4 address, or one IPv6 /64, as pairing does.</summary>
    public static string Caller(HttpContext context) => PairingStore.AddressKey(context.Connection.RemoteIpAddress);

    private static EntityRef Machine => new("ui", Environment.MachineName, "machine");

    // Every failure goes to the log. The activity log keeps 500 entries, so it only hears of the one
    // that engages a lock: a burst of guesses is one entry, not a flood.
    private async Task FailAsync(Reservation attempt, string caller, string what)
    {
        LogFailed(what, caller, attempt.Failures);
        if (attempt.Lock > TimeSpan.Zero)
        {
            await activity.RecordAsync(
                Machine,
                ActivityReason.SignInFailed,
                ActivityOutcome.Failed,
                $"{what} failed {attempt.Failures} times from {caller}, which is locked out for {(int)Math.Ceiling(attempt.Lock.TotalSeconds)} seconds",
                "Wrong password or setup code.");
        }
    }

    [LoggerMessage(Level = LogLevel.Warning, Message = "{What} failed from {Caller}: wrong password or setup code ({Failures} recent tries)")]
    private partial void LogFailed(string what, string caller, int failures);

    [LoggerMessage(Level = LogLevel.Information, Message = "The UI password was changed from {Caller}; other sessions are signed out")]
    private partial void LogPasswordChanged(string caller);
}
