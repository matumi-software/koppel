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

var builder = WebApplication.CreateBuilder(args);

// Runs as a Windows service when the installer registers it as one; a no-op otherwise.
builder.Host.UseWindowsService(options => options.ServiceName = "Koppel");

// HTTP serves the UI. HTTPS, with the server's own self-signed certificate, serves the token
// endpoint and the APIs remote clients call; clients pin the certificate's fingerprint.
var httpPort = builder.Configuration.GetValue("Ports:Http", 5240);
var httpsPort = builder.Configuration.GetValue("Ports:Https", 5241);
var httpAddress = ListenAddress.Resolve(builder.Configuration[ListenAddress.Setting], builder.Environment.IsDevelopment());
builder.WebHost.ConfigureKestrel(kestrel =>
{
    if (httpAddress is null)
    {
        kestrel.ListenAnyIP(httpPort);
    }
    else
    {
        kestrel.Listen(httpAddress, httpPort);
    }

    kestrel.ListenAnyIP(httpsPort, listen => listen.UseHttps(https =>
        https.ServerCertificateSelector = (_, _) => kestrel.ApplicationServices.GetRequiredService<ServerSecretsStore>().Certificate));
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
builder.Services.AddSingleton(new HostInfo(DateTimeOffset.UtcNow, httpPort, httpsPort));
builder.Services.AddSingleton(new ReservedPorts(httpPort, httpsPort, builder.Configuration.GetValue("Gateway:Port", OperatingSystem.IsWindows() ? UsbIpGateway.WindowsDefaultPort : 0)));
builder.Services.AddSingleton<InMemoryActivityStore>(sp => new InMemoryActivityStore(sp.GetRequiredService<TimeProvider>()));
builder.Services.AddSingleton<IActivityRecorder>(sp => sp.GetRequiredService<InMemoryActivityStore>());
builder.Services.AddSingleton<IActivityReader>(sp => sp.GetRequiredService<InMemoryActivityStore>());
builder.Services.AddSingleton(sp => SecretBoxFactory.Create(paths, sp.GetRequiredService<ILoggerFactory>().CreateLogger("Koppel.Security")));
SecretBoxFactory.AddAppDataProtection(builder.Services, paths);

builder.Services.AddSingleton<SettingsStore>();
builder.Services.AddSingleton<ServerSecretsStore>();
builder.Services.AddSingleton<ConnectionStore>();
builder.Services.AddSingleton<UiAuthStore>();
builder.Services.AddSingleton<UiLoginThrottle>();
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
builder.Services.AddSingleton<UsbIpTlsServer>();
builder.Services.AddHostedService(sp => sp.GetRequiredService<UsbIpTlsServer>());
builder.Services.AddHostedService<UsbIpGateway>();
builder.Services.AddSingleton<PairingStore>();
builder.Services.AddSingleton<PairingClient>();
builder.Services.AddSingleton<ServerAnnouncer>();
builder.Services.AddSingleton<ServerBrowser>();
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

// Static files before routing, so the SPA assets never hit an endpoint.
app.UseStaticFiles();
app.UseAuthentication();
app.UseAuthorization();

// Last guard before the routes: every UI API route and the hub need the session cookie (Surface.RequiresSignIn).
app.UseUiAuthGuard();

app.MapUiAuthEndpoints();
app.MapSettingsEndpoints();
app.MapServerEndpoints(httpsPort);
app.MapServerPairingEndpoints();
app.MapPairingEndpoints();
app.MapRemoteEndpoints().MapUsbRemoteEndpoints();
app.MapClientEndpoints();
app.MapUsbServerEndpoints();
app.MapUsbClientEndpoints();
app.MapSystemEndpoints();
app.MapActivityEndpoints();
app.MapHub<UiHub>("/api/hubs/ui");
app.MapHub<RemoteHub>("/api/hubs/remote", options => options.CloseOnAuthenticationExpiration = true);

// For a container's health check: anonymous, on the UI port, whatever the role.
app.MapGet("/healthz", () => TypedResults.Text("ok"));

app.MapFallbackToFile("index.html");

app.Run();
return 0;
