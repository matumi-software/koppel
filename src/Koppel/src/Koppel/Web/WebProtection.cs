using System.Threading.RateLimiting;
using Koppel.Login;

namespace Koppel.Web;

/// <summary>Response headers and rate limits that guard the whole host.</summary>
internal static class WebProtection
{
    /// <summary>Setup, login and password change: each one is a password guess or a hash, so far fewer than a person needs.</summary>
    private const int DefaultAuthPerMinute = 30;

    /// <summary>
    /// No framing (clickjacking on one-click Share and Revoke), no content sniffing, no referrer.
    /// Only <c>frame-ancestors</c> in the CSP: it cannot break the SPA or the Vite dev proxy.
    /// </summary>
    public static IApplicationBuilder UseSecurityHeaders(this IApplicationBuilder app) => app.Use(async (context, next) =>
    {
        var headers = context.Response.Headers;
        headers.XFrameOptions = "DENY";
        headers.ContentSecurityPolicy = "frame-ancestors 'none'";
        headers.XContentTypeOptions = "nosniff";
        headers["Referrer-Policy"] = "no-referrer";
        await next(context);
    });

    /// <summary>
    /// Rate limits by caller on the UI's setup, login and password routes, which a stranger on the network
    /// can reach before signing in. Over the limit answers 429. Skakel limits its own token and pairing
    /// endpoints (<c>RateLimits:TokenPerMinute</c> and <c>RateLimits:PairPerMinute</c>, set in Program.cs).
    /// </summary>
    public static IServiceCollection AddKoppelRateLimits(this IServiceCollection services, IConfiguration configuration)
    {
        var auth = configuration.GetValue("RateLimits:AuthPerMinute", DefaultAuthPerMinute);
        return services.AddRateLimiter(options =>
        {
            options.RejectionStatusCode = StatusCodes.Status429TooManyRequests;
            options.GlobalLimiter = PartitionedRateLimiter.Create<HttpContext, string>(context =>
            {
                var path = context.Request.Path;

                // Counted per address, as the sign-in throttle is. State and logout are left out: the UI polls state.
                return AuthPaths.Any(p => path.StartsWithSegments(p))
                    ? Window($"auth:{UiSignIn.Caller(context)}", auth)
                    : RateLimitPartition.GetNoLimiter("none");
            });
        });
    }

    private static readonly string[] AuthPaths = ["/api/auth/setup", "/api/auth/login", "/api/auth/password"];

    private static RateLimitPartition<string> Window(string key, int permits) =>
        RateLimitPartition.GetFixedWindowLimiter(key, _ => new FixedWindowRateLimiterOptions
        {
            PermitLimit = permits,
            Window = TimeSpan.FromMinutes(1),
            QueueLimit = 0,
        });
}
