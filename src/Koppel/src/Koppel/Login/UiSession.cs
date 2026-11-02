using System.Security.Claims;
using Microsoft.AspNetCore.Authentication;
using Microsoft.AspNetCore.Authentication.Cookies;

namespace Koppel.Login;

/// <summary>The UI's cookie sign-in: the scheme, the claims it carries and how a session is checked on every request.</summary>
internal static class UiSession
{
    public const string Scheme = "koppel.ui";
    public const string CookieName = "koppel.ui";
    public const string StampClaim = "koppel:stamp";
    public const string SessionClaim = "koppel:session";

    /// <summary>How long a session lasts without use. Each request in the second half of it extends it.</summary>
    public static readonly TimeSpan Lifetime = TimeSpan.FromHours(12);

    /// <summary>
    /// Cookie authentication for the UI surfaces only. The remote surfaces never look at it: they keep
    /// bearer tokens. An API request that is not signed in gets 401, never a redirect.
    /// </summary>
    public static IServiceCollection AddUiSession(this IServiceCollection services)
    {
        services.AddAuthentication().AddCookie(Scheme, options =>
        {
            options.Cookie.Name = CookieName;
            options.Cookie.HttpOnly = true;
            options.Cookie.SameSite = SameSiteMode.Strict;
            options.Cookie.SecurePolicy = CookieSecurePolicy.SameAsRequest;
            options.ExpireTimeSpan = Lifetime;
            options.SlidingExpiration = true;
            options.Events.OnRedirectToLogin = context =>
            {
                context.Response.StatusCode = StatusCodes.Status401Unauthorized;
                return Task.CompletedTask;
            };
            options.Events.OnRedirectToAccessDenied = context =>
            {
                context.Response.StatusCode = StatusCodes.Status403Forbidden;
                return Task.CompletedTask;
            };
            options.Events.OnValidatePrincipal = ValidateStampAsync;
        });
        return services;
    }

    /// <summary>A new session: its own id, and the stamp in force.</summary>
    public static ClaimsPrincipal NewPrincipal(string stamp, string? sessionId = null) =>
        new(new ClaimsIdentity(
            [new Claim(SessionClaim, sessionId ?? Guid.NewGuid().ToString("N")), new Claim(StampClaim, stamp)],
            Scheme));

    public static string? SessionOf(ClaimsPrincipal principal) => principal.FindFirst(SessionClaim)?.Value;

    public static Task SignInAsync(HttpContext context, ClaimsPrincipal principal) =>
        context.SignInAsync(Scheme, principal, new AuthenticationProperties { IsPersistent = false });

    // A password change or reset gives every earlier cookie a stale stamp.
    private static async Task ValidateStampAsync(CookieValidatePrincipalContext context)
    {
        var store = context.HttpContext.RequestServices.GetRequiredService<UiAuthStore>();
        if (context.Principal?.FindFirst(StampClaim)?.Value != store.SecurityStamp || !store.HasPassword)
        {
            context.RejectPrincipal();
            await context.HttpContext.SignOutAsync(Scheme);
        }
    }
}
