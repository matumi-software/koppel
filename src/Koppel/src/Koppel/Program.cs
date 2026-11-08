using Koppel;
using Koppel.Client;
using Koppel.Core.Activity;
using Koppel.Core.Usb;
using Koppel.Core.Usb.Linux;
using Koppel.Core.Usb.Windows;
using Koppel.Discovery;
using Koppel.Hubs;
using Koppel.Login;
using Koppel.Security;
using Koppel.Server;
using Koppel.Settings;
using Koppel.Tray;
using Koppel.Usb;
using Koppel.Web;
using Microsoft.AspNetCore.Server.Kestrel.Core;
using Microsoft.Extensions.Hosting.WindowsServices;
using Skakel.Client;
using Skakel.Server;

// The tray icon the Windows service starts in each signed-in session (TrayLauncher), and the Start menu entry that opens
// the UI (KoppelOpener). Each is a separate process that needs none of what follows.
if (OperatingSystem.IsWindows() && TrayIcon.Requested(args))
{
    return TrayIcon.Run(args);
}

if (OperatingSystem.IsWindows() && KoppelOpener.Requested(args))
{
    return KoppelOpener.Run(args);
}

// What the service starts as it stops for the tray's Restart Koppel: it starts the service again (ServiceRestart).
if (OperatingSystem.IsWindows() && ServiceRestart.Requested(args))
{
    return ServiceRestart.Run();
}

var builder = WebApplication.CreateBuilder(args);

// Runs as a Windows service when the installer registers it as one, and as a systemd service (readiness and
// journal log levels) when the Linux package starts it; each is a no-op otherwise. As a Windows service it also
// takes the tray icon's shut down control (KoppelServiceLifetime).
builder.Host.UseWindowsService(options => options.ServiceName = KoppelService.Name);
builder.Host.UseSystemd();
var windowsService = OperatingSystem.IsWindows() && WindowsServiceHelpers.IsWindowsService();
if (OperatingSystem.IsWindows() && windowsService)
{
    builder.Services.AddSingleton<IHostLifetime, KoppelServiceLifetime>();
}

// One HTTPS port, 5240, with the server's own self-signed certificate, serves the UI and everything other
// Koppel machines do through Skakel, USB traffic included; clients pin the certificate's fingerprint. Ports:Network moves
// the second onto a port of its own, and discovery answers on UDP at the network port's number unless
// Discovery:Port says otherwise. Ui:AllowFrom says who may use the UI (UiAccess), and Ui:SetupWindowMinutes how
// long a machine without a password takes its first one (UiSetupWindow).
var uiPort = builder.Configuration.GetValue("Ports:Ui", 5240);
var networkPort = builder.Configuration.GetValue("Ports:Network", uiPort);
var discoveryPort = builder.Configuration.GetValue("Discovery:Port", networkPort);
var listenAddress = ListenAddress.Resolve(builder.Configuration[ListenAddress.Setting]);
var uiReach = UiAccess.Resolve(builder.Configuration[UiAccess.Setting], builder.Environment.IsDevelopment());
var setupWindow = UiSetupWindow.Resolve(builder.Configuration[UiSetupWindow.Setting]);

// The tray icon in each signed-in session, only when Koppel runs as the installed Windows service. It opens the UI
// at the address Kestrel listens on, localhost unless Listen:Address names one. The Start menu entry reads the same
// address from the registry. The launcher is registered further down, after the stores its icons report on.
string? trayUiUrl = null;
if (OperatingSystem.IsWindows() && windowsService)
{
    var uiHost = listenAddress is null || listenAddress.Equals(System.Net.IPAddress.Any) || listenAddress.Equals(System.Net.IPAddress.IPv6Any)
        ? "localhost"
        : listenAddress.ToString();
    trayUiUrl = new UriBuilder(Uri.UriSchemeHttps, uiHost, uiPort).Uri.AbsoluteUri;
    KoppelOpener.PublishUiUrl(trayUiUrl);
}

// A one-off command for the Docker health check: ask the UI port whether Koppel answers, and exit.
if (HealthProbe.Requested(args))
{
    return await HealthProbe.RunAsync(listenAddress, uiPort);
}

builder.WebHost.ConfigureKestrel(kestrel =>
{
    // Plain HTTP on the port gets a redirect to HTTPS rather than nothing (PlainHttpRedirect). The certificate is
    // Skakel's, the current one at each handshake, so a switch takes effect at once.
    void Https(ListenOptions listen) => listen.UsePlainHttpRedirect().UseHttps(https =>
        https.UseSkakelCertificate(kestrel.ApplicationServices));

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
    return await UiPasswordReset.RunAsync(paths, resetBox, setupWindow, Console.Out);
}

builder.Services.AddSingleton(paths);
builder.Services.AddSingleton(TimeProvider.System);
builder.Services.AddSingleton(new HostInfo(DateTimeOffset.UtcNow, uiPort, networkPort, discoveryPort, windowsService));
builder.Services.AddSingleton(new UiAccess(uiReach));
builder.Services.AddSingleton(new UiSetupWindow(setupWindow, $"https://{Environment.MachineName}:{uiPort}"));
builder.Services.AddSingleton<InMemoryActivityStore>(sp => new InMemoryActivityStore(sp.GetRequiredService<TimeProvider>(), logger: sp.GetRequiredService<ILogger<InMemoryActivityStore>>()));
builder.Services.AddSingleton<IActivityRecorder>(sp => sp.GetRequiredService<InMemoryActivityStore>());
builder.Services.AddSingleton<IActivityReader>(sp => sp.GetRequiredService<InMemoryActivityStore>());
builder.Services.AddSingleton(sp => SecretBoxFactory.Create(paths, sp.GetRequiredService<ILoggerFactory>().CreateLogger("Koppel.Security")));
SecretBoxFactory.AddAppDataProtection(builder.Services, paths);

// Skakel: pairing, tokens, the certificate and its handover, the remote hub and the streams. Its state is kept
// in the encrypted secrets.json. Added before Koppel's own services, which read the certificate when they start.
// Koppel answers discovery itself (ServerAnnouncerService), because it answers only while it is a server.
builder.Services.AddSingleton<ServerSecretsStore>();
builder.Services.AddSingleton<ISkakelStore>(sp => sp.GetRequiredService<ServerSecretsStore>());
builder.Services.AddSkakelServer(options =>
{
    options.AppId = SystemEndpoints.AppId;
    options.AppVersion = SystemEndpoints.Version;
    options.Port = networkPort;
    options.DiscoveryEnabled = false;
    options.KeepClientSecrets = true;
    options.TokenRequestsPerMinute = builder.Configuration.GetValue("RateLimits:TokenPerMinute", 60);
    options.PairRequestsPerMinute = builder.Configuration.GetValue("RateLimits:PairPerMinute", 120);
});

builder.Services.AddSingleton<SettingsStore>();
builder.Services.AddSingleton<ConnectionStore>();
builder.Services.AddSingleton<UiAuthStore>();
builder.Services.AddSingleton<UiLoginThrottle>();
builder.Services.AddSingleton<UiPasswordGate>();
builder.Services.AddSingleton<UiRevokedSessions>();
builder.Services.AddSingleton<UiSignIn>();
builder.Services.AddSingleton<UiConnections>();
builder.Services.AddUiSession();
builder.Services.AddSingleton<RemoteServer>();
builder.Services.AddHostedService(sp => sp.GetRequiredService<SettingsStore>());
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
builder.Services.AddHostedService<PairingActivity>();
builder.Services.AddSingleton<PairingClient>();
builder.Services.AddSingleton<ServerAnnouncer>();
// Clients look for Koppel servers on their own discovery port, 5240 unless changed. The links follow a server that moved through the same browser.
builder.Services.AddSingleton(sp => new SkakelBrowser(discoveryPort, [SystemEndpoints.AppId], new SkakelClientOptions
{
    LoggerFactory = sp.GetRequiredService<ILoggerFactory>(),
    TimeProvider = sp.GetRequiredService<TimeProvider>(),
}));
builder.Services.AddSingleton<ServerFinder>();
builder.Services.AddHostedService<ServerAnnouncerService>();
builder.Services.AddHostedService(sp => sp.GetRequiredService<ServerFinder>());
builder.Services.AddHostedService<UiHubRelay>();

// Hosted services start in the order they are added, so the tray's feed starts after the stores it listens to.
if (OperatingSystem.IsWindows() && trayUiUrl is not null)
{
    TrayLauncher.Add(builder.Services, trayUiUrl);
}

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

// For other Koppel machines, on the network port: Skakel's info, pairing, tokens and hello, the remote hub,
// the usb stream type, and Koppel's own remote endpoints behind a client's token.
app.MapSkakel();
app.MapSkakelHub<RemoteHub>(RemoteHub.Path);
app.MapUsbStream();
app.MapRemoteEndpoints();
app.MapClientEndpoints();
app.MapUsbServerEndpoints();
app.MapUsbClientEndpoints();
app.MapSystemEndpoints();
app.MapActivityEndpoints();
app.MapHub<UiHub>("/api/hubs/ui", options => options.CloseOnAuthenticationExpiration = true);

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
