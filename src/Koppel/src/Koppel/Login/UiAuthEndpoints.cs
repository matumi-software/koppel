using System.Globalization;
using Microsoft.AspNetCore.Authentication;

namespace Koppel.Login;

internal sealed record UiAuthState(bool SignedIn, bool PasswordSet, string MachineName);

internal sealed record SetupRequest(string? SetupCode, string? Password);

internal sealed record LoginRequest(string? Password);

internal sealed record ChangePasswordRequest(string? Current, string? Next);

internal static class UiAuthEndpoints
{
    /// <summary>
    /// Sign-in for the UI. State, setup and login are anonymous (see <c>Surface.Anonymous</c>); logout
    /// and password change need the session. Success is 204 with a cookie, a wrong password or code is
    /// 401, 403 or 400 with <c>{ error }</c>, and a caller who tried too often gets 429 with Retry-After.
    /// </summary>
    public static IEndpointRouteBuilder MapUiAuthEndpoints(this IEndpointRouteBuilder app)
    {
        var group = app.MapGroup("/api/auth");

        group.MapGet("/state", async (HttpContext context, UiAuthStore store) =>
        {
            var session = await context.AuthenticateAsync(UiSession.Scheme);
            return TypedResults.Ok(new UiAuthState(session.Succeeded, store.HasPassword, Environment.MachineName));
        });

        group.MapPost("/setup", async Task<IResult> (SetupRequest request, HttpContext context, UiAuthStore store, UiSignIn signIn, CancellationToken cancellationToken) =>
        {
            var result = await signIn.SetupAsync(UiSignIn.Caller(context), request.SetupCode, request.Password, cancellationToken);
            if (result.Verdict == Verdict.Ok)
            {
                await UiSession.SignInAsync(context, UiSession.NewPrincipal(store.SecurityStamp));
            }

            return Answer(context, result, wrong: StatusCodes.Status403Forbidden, wrongText: "That setup code is not right.");
        });

        group.MapPost("/login", async Task<IResult> (LoginRequest request, HttpContext context, UiAuthStore store, UiSignIn signIn) =>
        {
            var result = await signIn.LoginAsync(UiSignIn.Caller(context), request.Password);
            if (result.Verdict == Verdict.Ok)
            {
                await UiSession.SignInAsync(context, UiSession.NewPrincipal(store.SecurityStamp));
            }

            return Answer(context, result, wrong: StatusCodes.Status401Unauthorized, wrongText: "That password is not right.");
        });

        group.MapPost("/logout", async (HttpContext context, UiConnections connections) =>
        {
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
                await UiSession.SignInAsync(context, UiSession.NewPrincipal(store.SecurityStamp, session));
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
            case Verdict.Invalid:
                return TypedResults.Json(new { error = result.Problem }, statusCode: StatusCodes.Status400BadRequest);
            case Verdict.AlreadySet:
                return TypedResults.Json(new { error = "A password is already set." }, statusCode: StatusCodes.Status409Conflict);
            case Verdict.NotSet:
                return TypedResults.Json(new { error = "No password is set yet. Set one with the setup code." }, statusCode: StatusCodes.Status409Conflict);
            default:
                return TypedResults.Json(new { error = wrongText }, statusCode: wrong);
        }
    }

    private static string Wait(int seconds) => seconds >= 90 ? $"{(seconds + 59) / 60} minutes" : seconds == 1 ? "1 second" : $"{seconds} seconds";
}
