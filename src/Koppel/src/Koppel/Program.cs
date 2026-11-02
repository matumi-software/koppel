using Koppel;
using Koppel.Client;
using Koppel.Core.Activity;
using Koppel.Core.Auth;
using Koppel.Core.Usb;
using Koppel.Core.Usb.Linux;
using Koppel.Hubs;
using Koppel.Security;
using Koppel.Server;
using Koppel.Settings;
using Koppel.Usb;

var builder = WebApplication.CreateBuilder(args);

// HTTP serves the UI. HTTPS, with the server's own self-signed certificate, serves the token
// endpoint and the APIs remote clients call; clients pin the certificate's fingerprint.
var httpPort = builder.Configuration.GetValue("Ports:Http", 5240);
var httpsPort = builder.Configuration.GetValue("Ports:Https", 5241);
builder.WebHost.ConfigureKestrel(kestrel =>
{
    kestrel.ListenAnyIP(httpPort);
    kestrel.ListenAnyIP(httpsPort, listen => listen.UseHttps(https =>
        https.ServerCertificateSelector = (_, _) => kestrel.ApplicationServices.GetRequiredService<ServerSecretsStore>().Certificate));
});

builder.Services.ConfigureHttpJsonOptions(options => KoppelJson.Configure(options.SerializerOptions));
builder.Services.AddSignalR().AddJsonProtocol(options => KoppelJson.Configure(options.PayloadSerializerOptions));

var paths = KoppelPaths.FromConfiguration(builder.Configuration, builder.Environment);
builder.Services.AddSingleton(paths);
builder.Services.AddSingleton(TimeProvider.System);
builder.Services.AddSingleton(new HostInfo(DateTimeOffset.UtcNow, httpPort, httpsPort));
builder.Services.AddSingleton<InMemoryActivityStore>(sp => new InMemoryActivityStore(sp.GetRequiredService<TimeProvider>()));
builder.Services.AddSingleton<IActivityRecorder>(sp => sp.GetRequiredService<InMemoryActivityStore>());
builder.Services.AddSingleton<IActivityReader>(sp => sp.GetRequiredService<InMemoryActivityStore>());
builder.Services.AddSingleton(sp => SecretBoxFactory.Create(paths, sp.GetRequiredService<ILoggerFactory>().CreateLogger("Koppel.Security")));
SecretBoxFactory.AddAppDataProtection(builder.Services, paths);

builder.Services.AddSingleton<SettingsStore>();
builder.Services.AddSingleton<ServerSecretsStore>();
builder.Services.AddSingleton<ConnectionStore>();
builder.Services.AddSingleton<IClientCredentialValidator>(sp => sp.GetRequiredService<ServerSecretsStore>());
builder.Services.AddSingleton<ISigningKeySource>(sp => sp.GetRequiredService<ServerSecretsStore>());
builder.Services.AddSingleton<RemoteServer>();
builder.Services.AddHostedService(sp => sp.GetRequiredService<SettingsStore>());
builder.Services.AddHostedService(sp => sp.GetRequiredService<ServerSecretsStore>());
builder.Services.AddHostedService(sp => sp.GetRequiredService<ConnectionStore>());
builder.Services.AddSingleton<UsbServer>();
builder.Services.AddSingleton<UsbClient>();
builder.Services.AddHostedService(sp => sp.GetRequiredService<UsbServer>());
builder.Services.AddHostedService(sp => sp.GetRequiredService<UsbClient>());
builder.Services.AddHostedService<UsbIpTlsServer>();
builder.Services.AddHostedService<UiHubRelay>();

// USB goes through the Linux kernel's USB/IP drivers. Other platforms report why they cannot share yet.
var sysRoot = builder.Configuration["Usb:SysRoot"] ?? "/sys";
if (OperatingSystem.IsLinux())
{
    builder.Services.AddSingleton<IUsbHost>(new SysfsUsbHost(sysRoot));
    builder.Services.AddSingleton<IUsbAttacher>(new SysfsVhci(sysRoot));
}
else
{
    var unsupported = new UnsupportedUsb("Sharing and attaching USB devices needs Linux for now. Run the Koppel Docker image on a Linux host; Windows support comes later.");
    builder.Services.AddSingleton<IUsbHost>(unsupported);
    builder.Services.AddSingleton<IUsbAttacher>(unsupported);
}

builder.Services.AddKoppelAuthServer();

var app = builder.Build();

if (app.Environment.IsDevelopment())
{
    app.UseDeveloperExceptionPage();
}

// Static files before routing, so the SPA assets never hit an endpoint.
app.UseStaticFiles();
app.UseAuthentication();
app.UseAuthorization();

app.MapSettingsEndpoints();
app.MapServerEndpoints(httpsPort);
app.MapRemoteEndpoints().MapUsbRemoteEndpoints();
app.MapClientEndpoints();
app.MapUsbServerEndpoints();
app.MapUsbClientEndpoints();
app.MapSystemEndpoints();
app.MapActivityEndpoints();
app.MapHub<UiHub>("/api/hubs/ui");
app.MapHub<RemoteHub>("/api/hubs/remote");

app.MapFallbackToFile("index.html");

app.Run();
