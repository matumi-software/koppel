using System.Globalization;
using Microsoft.AspNetCore.Authentication;

namespace Koppel.Login;

/// <param name="SetupOpen">True while no password is set and the first one may still be set.</param>
/// <param name="SetupUntil">While no password is set: when setup closes, or closed. Null once there is a password.</param>
internal sealed record UiAuthState(bool SignedIn, bool PasswordSet, string MachineName, bool SetupOpen, DateTimeOffset? SetupUntil);

internal sealed record SetupRequest(string? Password);

internal sealed record LoginRequest(string? Password);

internal sealed record ChangePasswordRequest(string? Current, string? Next);

internal static class UiAuthEndpoints
{
    /// <summary>
    /// Sign-in for the UI. State, setup and login are anonymous (see <c>Surface.Anonymous</c>); logout
    /// and password change need the session. Success is 204 with a cookie, a wrong password is 401 or 400
    /// and setup after its window 403, each with <c>{ error }</c>, and a caller who tried too often, or
    /// arrives while the server is busy hashing other passwords, gets 429 with Retry-After.
    /// </summary>
    public static IEndpointRouteBuilder MapUiAuthEndpoints(this IEndpointRouteBuilder app)
    {
        var group = app.MapGroup("/api/auth");

        group.MapGet("/state", async (HttpContext context, UiAuthStore store) =>
        {
            var session = await context.AuthenticateAsync(UiSession.Scheme);
            return TypedResults.Ok(new UiAuthState(session.Succeeded, store.HasPassword, Environment.MachineName, store.SetupOpen, store.SetupUntil));
        });

        group.MapPost("/setup", async Task<IResult> (SetupRequest request, HttpContext context, UiAuthStore store, UiSignIn signIn, TimeProvider time, CancellationToken cancellationToken) =>
        {
            var result = await signIn.SetupAsync(UiSignIn.Caller(context), request.Password, cancellationToken);
            if (result.Verdict == Verdict.Ok)
            {
                await UiSession.SignInAsync(context, UiSession.NewPrincipal(store.SecurityStamp, time.GetUtcNow()));
            }

            // Setup has no secret, so it is never wrong: a refusal is Invalid, AlreadySet or SetupClosed.
            return Answer(context, result, wrong: StatusCodes.Status400BadRequest, wrongText: "The password was not set.");
        });

        group.MapPost("/login", async Task<IResult> (LoginRequest request, HttpContext context, UiAuthStore store, UiSignIn signIn, TimeProvider time) =>
        {
            var result = await signIn.LoginAsync(UiSignIn.Caller(context), request.Password);
            if (result.Verdict == Verdict.Ok)
            {
                await UiSession.SignInAsync(context, UiSession.NewPrincipal(store.SecurityStamp, time.GetUtcNow()));
            }

            return Answer(context, result, wrong: StatusCodes.Status401Unauthorized, wrongText: "That password is not right.");
        });

        group.MapPost("/logout", async (HttpContext context, UiConnections connections, UiRevokedSessions revoked, CancellationToken cancellationToken) =>
        {
            // The cookie is only removed from this browser; revoking the id, which is written to
            // ui-auth.json so it holds across a restart, is what makes a copy of it useless.
            await revoked.RevokeAsync(UiSession.SessionOf(context.User), UiSession.EndOf(context.User), cancellationToken);
            connections.CloseSession(UiSession.SessionOf(context.User));
            await context.SignOutAsync(UiSession.Scheme);
            return TypedResults.NoContent();
        });

        // Completes on this response: unlike a setting there is no value to push back, and the new
        // password is never shown again. The confirm rule is for state the server owns and echoes.
        group.MapPost("/password", async Task<IResult> (ChangePasswordRequest request, HttpContext context, UiAuthStore store, UiSignIn signIn, CancellationToken cancellationToken) =>
        {
            var session = UiSession.SessionOf(context.User)!;
            var result = await signIn.ChangePasswordAsync(UiSignIn.Caller(context), request.Current, request.Next, session, cancellationToken);
            if (result.Verdict == Verdict.Ok)
            {
                // The change moved the stamp; this session gets a cookie for the new one.
                await UiSession.SignInAsync(context, UiSession.NewPrincipal(store.SecurityStamp, UiSession.IssuedOf(context.User)!.Value, session));
            }

            return Answer(context, result, wrong: StatusCodes.Status400BadRequest, wrongText: "The current password is not right.");
        });

        return app;
    }

    private static IResult Answer(HttpContext context, SignInResult result, int wrong, string wrongText)
    {
        switch (result.Verdict)
        {
            case Verdict.Ok:
                return TypedResults.NoContent();
            case Verdict.Locked:
                var seconds = (int)Math.Ceiling(result.RetryAfter!.Value.TotalSeconds);
                context.Response.Headers.RetryAfter = seconds.ToString(CultureInfo.InvariantCulture);
                return TypedResults.Json(new { error = $"Too many tries. Wait {Wait(seconds)} and try again." }, statusCode: StatusCodes.Status429TooManyRequests);
            case Verdict.Busy:
                context.Response.Headers.RetryAfter = "1";
                return TypedResults.Json(new { error = "The server is busy checking another password. Try again in a moment." }, statusCode: StatusCodes.Status429TooManyRequests);
            case Verdict.Invalid:
                return TypedResults.Json(new { error = result.Problem }, statusCode: StatusCodes.Status400BadRequest);
            case Verdict.SetupClosed:
                return TypedResults.Json(new { error = result.Problem }, statusCode: StatusCodes.Status403Forbidden);
            case Verdict.AlreadySet:
                return TypedResults.Json(new { error = "A password is already set." }, statusCode: StatusCodes.Status409Conflict);
            case Verdict.NotSet:
                return TypedResults.Json(new { error = "No password is set yet. Open the UI to set one." }, statusCode: StatusCodes.Status409Conflict);
            default:
                return TypedResults.Json(new { error = wrongText }, statusCode: wrong);
        }
    }

    private static string Wait(int seconds) => seconds >= 90 ? $"{(seconds + 59) / 60} minutes" : seconds == 1 ? "1 second" : $"{seconds} seconds";
}
