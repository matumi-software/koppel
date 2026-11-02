using System.Threading.RateLimiting;
using Koppel.Core.Auth;
using Koppel.Server;

namespace Koppel.Web;

/// <summary>Response headers and rate limits that guard the whole host.</summary>
internal static class WebProtection
{
    /// <summary>Real clients ask for a token about every 30 minutes and on each attach, so this is far above use.</summary>
    private const int DefaultTokenPerMinute = 60;

    /// <summary>Setup, login and password change: each one is a password guess or a hash, so far fewer than a person needs.</summary>
    private const int DefaultAuthPerMinute = 30;

    /// <summary>Pairing polls every 2 seconds, 30 a minute for each client waiting.</summary>
    private const int DefaultPairPerMinute = 120;

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
    /// Rate limits by remote IP on the token endpoint, the pairing endpoints and the UI's setup, login
    /// and password routes, the ones a stranger on the network can reach before it has credentials.
    /// Over the limit answers 429.
    /// </summary>
    public static IServiceCollection AddKoppelRateLimits(this IServiceCollection services, IConfiguration configuration)
    {
        var tokens = configuration.GetValue("RateLimits:TokenPerMinute", DefaultTokenPerMinute);
        var pairing = configuration.GetValue("RateLimits:PairPerMinute", DefaultPairPerMinute);
        var auth = configuration.GetValue("RateLimits:AuthPerMinute", DefaultAuthPerMinute);
        return services.AddRateLimiter(options =>
        {
            options.RejectionStatusCode = StatusCodes.Status429TooManyRequests;
            options.GlobalLimiter = PartitionedRateLimiter.Create<HttpContext, string>(context =>
            {
                var path = context.Request.Path;
                if (path.StartsWithSegments(KoppelAuthServer.TokenPath))
                {
                    return Window($"token:{RemoteAddress(context)}", tokens);
                }

                if (path.StartsWithSegments("/api/pair"))
                {
                    return Window($"pair:{RemoteAddress(context)}", pairing);
                }

                // Counted per IPv4 address or IPv6 /64, as the sign-in throttle is. State and logout are left out: the UI polls state.
                return AuthPaths.Any(p => path.StartsWithSegments(p))
                    ? Window($"auth:{PairingStore.AddressKey(context.Connection.RemoteIpAddress)}", auth)
                    : RateLimitPartition.GetNoLimiter("none");
            });
        });
    }

    private static readonly string[] AuthPaths = ["/api/auth/setup", "/api/auth/login", "/api/auth/password"];

    /// <summary>The caller's address, with an IPv4 address in its IPv6 form folded back to IPv4.</summary>
    internal static string RemoteAddress(HttpContext context) =>
        context.Connection.RemoteIpAddress is { } ip ? (ip.IsIPv4MappedToIPv6 ? ip.MapToIPv4() : ip).ToString() : "unknown";

    private static RateLimitPartition<string> Window(string key, int permits) =>
        RateLimitPartition.GetFixedWindowLimiter(key, _ => new FixedWindowRateLimiterOptions
        {
            PermitLimit = permits,
            Window = TimeSpan.FromMinutes(1),
            QueueLimit = 0,
        });
}
