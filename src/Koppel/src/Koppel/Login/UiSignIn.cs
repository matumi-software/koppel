using Koppel.Core.Activity;
using Koppel.Web;

namespace Koppel.Login;

internal enum Verdict
{
    Ok,
    Wrong,
    Locked,
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
internal sealed partial class UiSignIn(UiAuthStore store, UiLoginThrottle throttle, IActivityRecorder activity, ILogger<UiSignIn> logger)
{
    private const string AnyCaller = "setup:any";

    public async Task<SignInResult> LoginAsync(string caller, string? password)
    {
        var key = $"login:{caller}";
        if (throttle.LockedFor(key) is { } wait)
        {
            return new SignInResult(Verdict.Locked, wait);
        }

        if (!store.HasPassword)
        {
            return new SignInResult(Verdict.NotSet);
        }

        if (store.VerifyPassword(password))
        {
            throttle.Clear(key);
            return new SignInResult(Verdict.Ok);
        }

        await FailAsync(key, caller, "Sign-in");
        return new SignInResult(Verdict.Wrong);
    }

    /// <summary>
    /// Sets the first password. The caller is limited, and so are all callers together: the code is
    /// short, and an attacker with many addresses must not get unlimited guesses.
    /// </summary>
    public async Task<SignInResult> SetupAsync(string caller, string? code, string? password, CancellationToken cancellationToken)
    {
        if (store.HasPassword)
        {
            return new SignInResult(Verdict.AlreadySet);
        }

        var mine = $"setup:{caller}";
        var wait = throttle.LockedFor(mine) ?? throttle.LockedFor(AnyCaller);
        if (wait is not null)
        {
            return new SignInResult(Verdict.Locked, wait);
        }

        if (UiPassword.Validate(password) is { } problem)
        {
            return new SignInResult(Verdict.Invalid, Problem: problem);
        }

        switch (await store.SetFirstPasswordAsync(code, password!, cancellationToken))
        {
            case SetupOutcome.Done:
                throttle.Clear(mine);
                return new SignInResult(Verdict.Ok);
            case SetupOutcome.AlreadySet:
                return new SignInResult(Verdict.AlreadySet);
            default:
                throttle.Fail(AnyCaller);
                await FailAsync(mine, caller, "Setup with the setup code");
                return new SignInResult(Verdict.Wrong);
        }
    }

    public async Task<SignInResult> ChangePasswordAsync(string caller, string? current, string? next, string session, CancellationToken cancellationToken)
    {
        var key = $"login:{caller}";
        if (throttle.LockedFor(key) is { } wait)
        {
            return new SignInResult(Verdict.Locked, wait);
        }

        if (!store.VerifyPassword(current))
        {
            await FailAsync(key, caller, "Password change");
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

    public static string Caller(HttpContext context) => WebProtection.RemoteAddress(context);

    private static EntityRef Machine => new("ui", Environment.MachineName, "machine");

    private async Task FailAsync(string key, string caller, string what)
    {
        throttle.Fail(key);
        LogFailed(what, caller);
        await activity.RecordAsync(Machine, ActivityReason.SignInFailed, ActivityOutcome.Failed, $"{what} failed from {caller}", "Wrong password or setup code.");
    }

    [LoggerMessage(Level = LogLevel.Warning, Message = "{What} failed from {Caller}: wrong password or setup code")]
    private partial void LogFailed(string what, string caller);

    [LoggerMessage(Level = LogLevel.Information, Message = "The UI password was changed from {Caller}; other sessions are signed out")]
    private partial void LogPasswordChanged(string caller);
}
