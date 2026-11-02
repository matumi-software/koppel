using Koppel.Web;
using Microsoft.AspNetCore.Authentication;

namespace Koppel.Login;

/// <summary>
/// Answers 401 to a request for the UI's API or hub without a valid session cookie. What needs a
/// sign-in is decided by <see cref="Surface.RequiresSignIn"/>, not here.
/// </summary>
internal sealed class UiAuthGuard(RequestDelegate next)
{
    public async Task InvokeAsync(HttpContext context)
    {
        if (Surface.RequiresSignIn(context.Request.Path))
        {
            var result = await context.AuthenticateAsync(UiSession.Scheme);
            if (!result.Succeeded)
            {
                context.Response.StatusCode = StatusCodes.Status401Unauthorized;
                return;
            }

            context.User = result.Principal;
            // The cookie is not the default scheme, so tell SignalR when this one expires: that is what
            // lets the UI hub close with it (CloseOnAuthenticationExpiration).
            context.Features.Set<IAuthenticateResultFeature>(new SessionResult(result));
        }

        await next(context);
    }
}

internal sealed class SessionResult(AuthenticateResult result) : IAuthenticateResultFeature
{
    public AuthenticateResult? AuthenticateResult { get; set; } = result;
}

internal static class UiAuthGuardExtensions
{
    public static IApplicationBuilder UseUiAuthGuard(this IApplicationBuilder app) => app.UseMiddleware<UiAuthGuard>();
}
