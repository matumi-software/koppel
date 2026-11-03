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
    public const string IssuedClaim = "koppel:issued";

    /// <summary>How long a session lasts without use. Each request in the second half of it extends it.</summary>
    public static readonly TimeSpan Lifetime = TimeSpan.FromHours(12);

    /// <summary>How long a session lasts at most, however much it is used. Counted from the sign-in, and a password change does not restart it.</summary>
    public static readonly TimeSpan AbsoluteLifetime = TimeSpan.FromDays(7);

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
            // The UI is served over HTTPS only, and a browser never sends a Secure cookie over plain HTTP.
            options.Cookie.SecurePolicy = CookieSecurePolicy.Always;
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

    /// <summary>A session: its id (new unless it continues one), the stamp in force and when it was first signed in.</summary>
    public static ClaimsPrincipal NewPrincipal(string stamp, DateTimeOffset issued, string? sessionId = null) =>
        new(new ClaimsIdentity(
            [
                new Claim(SessionClaim, sessionId ?? Guid.NewGuid().ToString("N")),
                new Claim(StampClaim, stamp),
                new Claim(IssuedClaim, issued.ToUnixTimeSeconds().ToString(System.Globalization.CultureInfo.InvariantCulture)),
            ],
            Scheme));

    public static string? SessionOf(ClaimsPrincipal principal) => principal.FindFirst(SessionClaim)?.Value;

    /// <summary>When the session was first signed in, or null for a cookie that does not say.</summary>
    public static DateTimeOffset? IssuedOf(ClaimsPrincipal? principal) =>
        long.TryParse(principal?.FindFirst(IssuedClaim)?.Value, System.Globalization.NumberStyles.None, System.Globalization.CultureInfo.InvariantCulture, out var seconds)
            ? DateTimeOffset.FromUnixTimeSeconds(seconds)
            : null;

    /// <summary>The last moment the session could be valid, whatever else happens to it.</summary>
    public static DateTimeOffset EndOf(ClaimsPrincipal principal) => (IssuedOf(principal) ?? DateTimeOffset.UnixEpoch) + AbsoluteLifetime;

    public static Task SignInAsync(HttpContext context, ClaimsPrincipal principal) =>
        context.SignInAsync(Scheme, principal, new AuthenticationProperties { IsPersistent = false });

    // A password change or reset gives every earlier cookie a stale stamp. Signing out revokes the
    // session id, so a copy of the cookie dies with it, and no session outlives the absolute cap.
    private static async Task ValidateStampAsync(CookieValidatePrincipalContext context)
    {
        var services = context.HttpContext.RequestServices;
        var store = services.GetRequiredService<UiAuthStore>();
        if (context.Principal?.FindFirst(StampClaim)?.Value != store.SecurityStamp || !store.HasPassword
            || EndOf(context.Principal) <= services.GetRequiredService<TimeProvider>().GetUtcNow()
            || services.GetRequiredService<UiRevokedSessions>().IsRevoked(SessionOf(context.Principal)))
        {
            context.RejectPrincipal();
            await context.HttpContext.SignOutAsync(Scheme);
        }
    }
}
