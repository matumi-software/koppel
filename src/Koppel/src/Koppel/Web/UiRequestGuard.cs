namespace Koppel.Web;

/// <summary>
/// Protects the UI's API from other sites in the browser. A request to the UI's <c>/api/*</c>
/// is refused with 403 when its <c>Origin</c> is another origin, and a change (anything but GET or
/// HEAD) is refused without <c>X-Koppel-UI: 1</c>, a header a page on another site cannot add without
/// a CORS preflight this server never grants. The hub is exempt from the header only, because a
/// WebSocket upgrade cannot carry one; its Origin is still checked. The network surfaces (tokens,
/// pairing, the remote API) are not the browser's and are left alone, on whichever port they are.
/// </summary>
internal sealed class UiRequestGuard(RequestDelegate next)
{
    public const string HeaderName = "X-Koppel-UI";

    private static readonly PathString Hub = "/api/hubs/ui";

    public Task InvokeAsync(HttpContext context)
    {
        var request = context.Request;
        if (Surface.For(request.Path).Listener == Listener.Ui && request.Path.StartsWithSegments("/api") && !Allowed(request))
        {
            context.Response.StatusCode = StatusCodes.Status403Forbidden;
            return Task.CompletedTask;
        }

        return next(context);
    }

    private static bool Allowed(HttpRequest request)
    {
        if (request.Headers.TryGetValue("Origin", out var origin) && !SameOrigin(request, origin.ToString()))
        {
            return false;
        }

        var isChange = !HttpMethods.IsGet(request.Method) && !HttpMethods.IsHead(request.Method);
        return !isChange || request.Path.StartsWithSegments(Hub) || request.Headers[HeaderName] == "1";
    }

    /// <summary>
    /// The origin's scheme, host and port against this request's own. Behind a TLS proxy, such as the
    /// Vite dev server, the scheme comes from <c>X-Forwarded-Proto</c>; a browser cannot set that on a
    /// cross-site request without a preflight, so it is no way around the check.
    /// </summary>
    internal static bool SameOrigin(HttpRequest request, string origin)
    {
        if (!Uri.TryCreate(origin, UriKind.Absolute, out var uri))
        {
            return false;
        }

        var scheme = request.Headers["X-Forwarded-Proto"].FirstOrDefault() is { Length: > 0 } forwarded ? forwarded : request.Scheme;
        var port = request.Host.Port ?? (string.Equals(scheme, Uri.UriSchemeHttps, StringComparison.OrdinalIgnoreCase) ? 443 : 80);
        return string.Equals(uri.Scheme, scheme, StringComparison.OrdinalIgnoreCase)
            && string.Equals(uri.Host, request.Host.Host, StringComparison.OrdinalIgnoreCase)
            && uri.Port == port;
    }
}

internal static class UiRequestGuardExtensions
{
    public static IApplicationBuilder UseUiRequestGuard(this IApplicationBuilder app) => app.UseMiddleware<UiRequestGuard>();
}
