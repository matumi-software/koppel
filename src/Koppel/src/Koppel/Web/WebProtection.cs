using System.Threading.RateLimiting;
using Koppel.Core.Auth;

namespace Koppel.Web;

/// <summary>Response headers and rate limits that guard the whole host.</summary>
internal static class WebProtection
{
    /// <summary>Real clients ask for a token about every 30 minutes and on each attach, so this is far above use.</summary>
    private const int DefaultTokenPerMinute = 60;

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
    /// Rate limits by remote IP on the token endpoint and the pairing endpoints, the two a stranger on
    /// the network can reach before it has credentials. Over the limit answers 429.
    /// </summary>
    public static IServiceCollection AddKoppelRateLimits(this IServiceCollection services, IConfiguration configuration)
    {
        var tokens = configuration.GetValue("RateLimits:TokenPerMinute", DefaultTokenPerMinute);
        var pairing = configuration.GetValue("RateLimits:PairPerMinute", DefaultPairPerMinute);
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

                return path.StartsWithSegments("/api/pair")
                    ? Window($"pair:{RemoteAddress(context)}", pairing)
                    : RateLimitPartition.GetNoLimiter("none");
            });
        });
    }

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
