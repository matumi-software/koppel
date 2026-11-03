using Koppel;
using Koppel.Client;
using Koppel.Core.Activity;
using Koppel.Core.Auth;
using Koppel.Core.Discovery;
using Koppel.Core.Usb;
using Koppel.Core.Usb.Linux;
using Koppel.Core.Usb.Windows;
using Koppel.Discovery;
using Koppel.Hubs;
using Koppel.Login;
using Koppel.Security;
using Koppel.Server;
using Koppel.Settings;
using Koppel.Usb;
using Koppel.Web;
using Microsoft.AspNetCore.Server.Kestrel.Core;

var builder = WebApplication.CreateBuilder(args);

// Runs as a Windows service when the installer registers it as one; a no-op otherwise.
builder.Host.UseWindowsService(options => options.ServiceName = "Koppel");

// One HTTPS port, 5240, with the server's own self-signed certificate, serves the UI and everything other
// Koppel machines do, USB traffic included; clients pin the certificate's fingerprint. Ports:Network moves
// the second onto a port of its own, and discovery answers on UDP at the network port's number unless
// Discovery:Port says otherwise. Ui:AllowFrom says who may use the UI (UiAccess).
var uiPort = builder.Configuration.GetValue("Ports:Ui", 5240);
var networkPort = builder.Configuration.GetValue("Ports:Network", uiPort);
var discoveryPort = builder.Configuration.GetValue("Discovery:Port", networkPort);
var listenAddress = ListenAddress.Resolve(builder.Configuration[ListenAddress.Setting]);
var uiReach = UiAccess.Resolve(builder.Configuration[UiAccess.Setting], builder.Environment.IsDevelopment());

// A one-off command for the Docker health check: ask the UI port whether Koppel answers, and exit.
if (HealthProbe.Requested(args))
{
    return await HealthProbe.RunAsync(listenAddress, uiPort);
}

builder.WebHost.ConfigureKestrel(kestrel =>
{
    // Plain HTTP on the port gets a redirect to HTTPS rather than nothing (PlainHttpRedirect).
    void Https(ListenOptions listen) => listen.UsePlainHttpRedirect().UseHttps(https =>
        https.ServerCertificateSelector = (_, _) => kestrel.ApplicationServices.GetRequiredService<ServerSecretsStore>().Certificate);

    void Listen(int port)
    {
        if (listenAddress is null)
        {
            kestrel.ListenAnyIP(port, Https);
        }
        else
        {
            kestrel.Listen(listenAddress, port, Https);
        }
    }

    Listen(uiPort);
    if (networkPort != uiPort)
    {
        Listen(networkPort);
    }
});

builder.Services.AddKoppelRateLimits(builder.Configuration);
builder.Services.ConfigureHttpJsonOptions(options => KoppelJson.Configure(options.SerializerOptions));
builder.Services.AddSignalR().AddJsonProtocol(options => KoppelJson.Configure(options.PayloadSerializerOptions));

var paths = KoppelPaths.FromConfiguration(builder.Configuration, builder.Environment);
paths.Prepare();

// A one-off command: clear the UI password and exit, before anything else starts.
if (UiPasswordReset.Requested(args))
{
    using var resetBox = SecretBoxFactory.Create(paths, Microsoft.Extensions.Logging.Abstractions.NullLogger.Instance);
    return await UiPasswordReset.RunAsync(paths, resetBox, Console.Out);
}

builder.Services.AddSingleton(paths);
builder.Services.AddSingleton(TimeProvider.System);
builder.Services.AddSingleton(new HostInfo(DateTimeOffset.UtcNow, uiPort, networkPort, discoveryPort));
builder.Services.AddSingleton(new UiAccess(uiReach));
builder.Services.AddSingleton<InMemoryActivityStore>(sp => new InMemoryActivityStore(sp.GetRequiredService<TimeProvider>(), logger: sp.GetRequiredService<ILogger<InMemoryActivityStore>>()));
builder.Services.AddSingleton<IActivityRecorder>(sp => sp.GetRequiredService<InMemoryActivityStore>());
builder.Services.AddSingleton<IActivityReader>(sp => sp.GetRequiredService<InMemoryActivityStore>());
builder.Services.AddSingleton(sp => SecretBoxFactory.Create(paths, sp.GetRequiredService<ILoggerFactory>().CreateLogger("Koppel.Security")));
SecretBoxFactory.AddAppDataProtection(builder.Services, paths);

builder.Services.AddSingleton<SettingsStore>();
builder.Services.AddSingleton<ServerSecretsStore>();
builder.Services.AddSingleton<ConnectionStore>();
builder.Services.AddSingleton<UiAuthStore>();
builder.Services.AddSingleton<UiLoginThrottle>();
builder.Services.AddSingleton<UiPasswordGate>();
builder.Services.AddSingleton<UiRevokedSessions>();
builder.Services.AddSingleton<UiSignIn>();
builder.Services.AddSingleton<UiConnections>();
builder.Services.AddUiSession();
builder.Services.AddSingleton<IClientCredentialValidator>(sp => sp.GetRequiredService<ServerSecretsStore>());
builder.Services.AddSingleton<ISigningKeySource>(sp => sp.GetRequiredService<ServerSecretsStore>());
builder.Services.AddSingleton<RemoteServer>();
builder.Services.AddHostedService(sp => sp.GetRequiredService<SettingsStore>());
builder.Services.AddHostedService(sp => sp.GetRequiredService<ServerSecretsStore>());
builder.Services.AddHostedService(sp => sp.GetRequiredService<ConnectionStore>());
builder.Services.AddHostedService(sp => sp.GetRequiredService<UiAuthStore>());
builder.Services.AddHostedService(sp => sp.GetRequiredService<UiConnections>());
builder.Services.AddSingleton<UsbServer>();
builder.Services.AddSingleton<UsbClient>();
builder.Services.AddHostedService(sp => sp.GetRequiredService<UsbServer>());
builder.Services.AddHostedService(sp => sp.GetRequiredService<UsbClient>());
builder.Services.AddSingleton<LiveSessions>();
builder.Services.AddHostedService(sp => sp.GetRequiredService<LiveSessions>());
builder.Services.AddSingleton<UsbDataPlane>();
builder.Services.AddHostedService(sp => sp.GetRequiredService<UsbDataPlane>());
builder.Services.AddHostedService<UsbIpGateway>();
builder.Services.AddSingleton<PairingStore>();
builder.Services.AddSingleton<PairingClient>();
builder.Services.AddSingleton<ServerAnnouncer>();
// Clients look for servers on their own discovery port, 5240 unless changed.
builder.Services.AddSingleton(sp => new ServerBrowser(sp.GetRequiredService<TimeProvider>(), sp.GetRequiredService<ILogger<ServerBrowser>>(), discoveryPort));
builder.Services.AddSingleton<ServerFinder>();
builder.Services.AddHostedService<ServerAnnouncerService>();
builder.Services.AddHostedService(sp => sp.GetRequiredService<ServerFinder>());
builder.Services.AddHostedService<UiHubRelay>();

// USB goes through the Linux kernel's USB/IP drivers. Other platforms report why they cannot share yet.
// Usb:Upstream (host, host:port or [v6]:port) shares another USB/IP server's devices instead, such as usbipd-win on a
// Windows host; Gateway:Port lets a USB/IP driver on this machine attach (see UsbIpGateway).
var sysRoot = builder.Configuration["Usb:SysRoot"] ?? "/sys";
var unsupported = new UnsupportedUsb("Sharing and attaching USB devices needs Linux, or a USB/IP server or driver on the host. See docs/windows.md.");
if (builder.Configuration["Usb:Upstream"] is { Length: > 0 } upstream)
{
    var (upstreamHost, upstreamPort) = UpstreamAddress.Parse(upstream);
    builder.Services.AddSingleton<IUsbHost>(sp => new UsbIpUpstreamHost(upstreamHost, upstreamPort, sp.GetRequiredService<TimeProvider>()));
}
else
{
    builder.Services.AddSingleton<IUsbHost>(OperatingSystem.IsLinux() ? new SysfsUsbHost(sysRoot) : unsupported);
}

builder.Services.AddSingleton<IUsbAttacher>(OperatingSystem.IsLinux() ? new SysfsVhci(sysRoot) : unsupported);

// Windows attaches through usbip-win2's driver: the copy Koppel's installer bundles, or a standalone install.
var bundledUsbip = Path.Combine(AppContext.BaseDirectory, "usbip", "usbip.exe");
var usbipExe = builder.Configuration["Usb:UsbipPath"]
    ?? (File.Exists(bundledUsbip) ? bundledUsbip : Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ProgramFiles), "USBip", "usbip.exe"));
builder.Services.AddSingleton<IUsbIpDriver>(OperatingSystem.IsWindows() ? new UsbipWin2Driver(usbipExe) : unsupported);

builder.Services.AddKoppelAuthServer();

var app = builder.Build();

app.UseSecurityHeaders();
if (app.Environment.IsDevelopment())
{
    app.UseDeveloperExceptionPage();
}

// Port and role first: a request for a surface this listener or role does not have never reaches
// the rest. The rate limiter comes after, so such a request does not use up a caller's allowance.
app.UseSurfaceGuard();
app.UseUiRequestGuard();
app.UseRateLimiter();
app.UseWebSockets();

// Static files before routing, so the SPA assets never hit an endpoint.
app.UseStaticFiles();
app.UseAuthentication();
app.UseAuthorization();

// Last guard before the routes: every UI API route and the hub need the session cookie (Surface.RequiresSignIn).
app.UseUiAuthGuard();

app.MapUiAuthEndpoints();
app.MapSettingsEndpoints();
app.MapServerEndpoints(networkPort);
app.MapServerPairingEndpoints();
app.MapPairingEndpoints();
app.MapRemoteEndpoints().MapUsbRemoteEndpoints();
app.MapClientEndpoints();
app.MapUsbServerEndpoints();
app.MapUsbClientEndpoints();
app.MapSystemEndpoints();
app.MapActivityEndpoints();
app.MapHub<UiHub>("/api/hubs/ui", options => options.CloseOnAuthenticationExpiration = true);
app.MapHub<RemoteHub>("/api/hubs/remote", options => options.CloseOnAuthenticationExpiration = true);

// For a container's health check: anonymous, on the UI port, whatever the role.
app.MapGet("/healthz", () => TypedResults.Text("ok"));

app.MapFallbackToFile("index.html");

// A background service that dies stops the host, which would otherwise end as a clean stop and the service
// manager would not restart it. Say it failed: ServiceBase.ExitCode is what the manager is told when the
// service stops itself, Environment.ExitCode is the process exit code (a console, a container).
app.Lifetime.ApplicationStopping.Register(() =>
{
    var faulted = app.Services.GetServices<IHostedService>().OfType<BackgroundService>().Any(service => service.ExecuteTask?.IsFaulted == true);
    if (!faulted)
    {
        return;
    }

    Environment.ExitCode = 1;
    if (OperatingSystem.IsWindows() && app.Services.GetService<IHostLifetime>() is System.ServiceProcess.ServiceBase lifetime)
    {
        lifetime.ExitCode = 1;
    }
});

app.Run();
return Environment.ExitCode;
