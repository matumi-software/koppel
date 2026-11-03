using Koppel.Settings;

namespace Koppel.Web;

/// <summary>Which port a surface is served on. Both are the same port unless <c>Ports:Network</c> is set.</summary>
internal enum Listener
{
    /// <summary>The UI port: the people who run this machine.</summary>
    Ui,

    /// <summary>The network port: remote Koppel clients.</summary>
    Network,
}

/// <summary>Which role a surface belongs to. A machine that is both serves both.</summary>
internal enum RoleScope
{
    Any,
    Server,
    Client,
}

/// <summary>A path prefix, the listener that serves it and the role that needs it.</summary>
internal sealed record Surface(string Prefix, Listener Listener, RoleScope Role)
{
    /// <summary>Everything not listed below, the SPA and the UI's own API included.</summary>
    public static Surface Ui { get; } = new("/", Listener.Ui, RoleScope.Any);

    /// <summary>
    /// The one place that says what is served where. A request outside every prefix is the UI's, so a
    /// new route is never exposed to remote clients by accident.
    /// </summary>
    public static IReadOnlyList<Surface> Listed { get; } =
    [
        new("/connect/token", Listener.Network, RoleScope.Server),
        new("/api/remote", Listener.Network, RoleScope.Server),
        new("/api/hubs/remote", Listener.Network, RoleScope.Server),
        new("/api/pair", Listener.Network, RoleScope.Server),
        new("/api/server", Listener.Ui, RoleScope.Server),
        new("/api/client", Listener.Ui, RoleScope.Client),
    ];

    /// <summary>
    /// The UI paths a person may reach without signing in. Everything else of the UI's under <c>/api</c>,
    /// a route added tomorrow included, needs the session cookie. The SPA's files and
    /// <c>/healthz</c> are not under <c>/api</c>, so they stay open.
    /// </summary>
    public static IReadOnlyList<string> Anonymous { get; } =
    [
        "/api/auth/state",
        "/api/auth/setup",
        "/api/auth/login",
    ];

    /// <summary>True when a request for this path needs a UI session: it is the UI's API or hub and not listed as anonymous.</summary>
    public static bool RequiresSignIn(PathString path) =>
        For(path).Listener == Listener.Ui
        && path.StartsWithSegments("/api")
        && !Anonymous.Any(anonymous => path.Equals(anonymous, StringComparison.OrdinalIgnoreCase));

    public static Surface For(PathString path)
    {
        foreach (var surface in Listed)
        {
            if (path.StartsWithSegments(surface.Prefix))
            {
                return surface;
            }
        }

        return Ui;
    }

    public bool ServedTo(KoppelRole role) => Role switch
    {
        RoleScope.Server => role != KoppelRole.Client,
        RoleScope.Client => role != KoppelRole.Server,
        _ => true,
    };
}

/// <summary>
/// Answers 404 to a request that reached a port its surface is not on, or a surface this machine's
/// current role does not have, and 403 to a UI request from an address <see cref="UiAccess"/> turns
/// away. The port is the local port the connection arrived on, never the Host header, which the caller
/// picks. The role is read from the settings on every request, so a role change takes effect at once.
/// </summary>
internal sealed class SurfaceGuard(RequestDelegate next, HostInfo host, SettingsStore settings, UiAccess access)
{
    public Task InvokeAsync(HttpContext context)
    {
        var surface = Surface.For(context.Request.Path);
        var port = context.Connection.LocalPort;
        var onPort = port == (surface.Listener == Listener.Network ? host.NetworkPort : host.UiPort);
        if (!onPort || !surface.ServedTo(settings.Current.Role))
        {
            context.Response.StatusCode = StatusCodes.Status404NotFound;
            return Task.CompletedTask;
        }

        if (surface.Listener == Listener.Ui && !access.Allows(context.Connection.RemoteIpAddress))
        {
            context.Response.StatusCode = StatusCodes.Status403Forbidden;
            return context.Response.WriteAsync($"Koppel's UI does not answer this address. {UiAccess.Setting} decides who may use it.");
        }

        return next(context);
    }
}

internal static class SurfaceGuardExtensions
{
    public static IApplicationBuilder UseSurfaceGuard(this IApplicationBuilder app) => app.UseMiddleware<SurfaceGuard>();
}
