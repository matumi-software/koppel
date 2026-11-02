using Koppel;
using Koppel.Client;
using Koppel.Core.Auth;
using Koppel.Hubs;
using Koppel.Security;
using Koppel.Server;
using Koppel.Settings;

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
builder.Services.AddHostedService<UiHubRelay>();

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
app.MapRemoteEndpoints();
app.MapClientEndpoints();
app.MapHub<UiHub>("/api/hubs/ui");

app.MapFallbackToFile("index.html");

app.Run();
