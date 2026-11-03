using System.Net;
using System.Net.Http.Json;
using System.Net.Sockets;
using Koppel.Core.Usb;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Hosting;
using Microsoft.AspNetCore.Mvc.Testing;
using Microsoft.AspNetCore.SignalR.Client;
using Microsoft.AspNetCore.TestHost;
using Microsoft.Extensions.DependencyInjection;

namespace Koppel.Tests;

/// <summary>
/// Gives each request on a test server the local port its scheme stands for, as Kestrel does for a real
/// connection: the UI port for http, the HTTPS port for https. Both ports are HTTPS on a real host; a
/// test server has no TLS, so the scheme only tells the two apart.
/// </summary>
public sealed class TestListenerPorts(int httpPort, int httpsPort) : IStartupFilter
{
    public Action<IApplicationBuilder> Configure(Action<IApplicationBuilder> next) => app =>
    {
        app.Use((context, nextMiddleware) =>
        {
            context.Connection.LocalPort = context.Request.IsHttps ? httpsPort : httpPort;
            return nextMiddleware(context);
        });
        next(app);
    };
}

/// <summary>
/// A Koppel host with its own temporary config and key directories. By default it runs on a test
/// server. With <c>kestrel: true</c> it listens on real free ports, so two hosts can talk as server
/// and client, and the HTTPS ports can be reached. The UI and network ports are separate, as with
/// <c>Ports:Network</c> set, unless <c>singlePort</c>, the default outside tests. With <c>shareDongle</c>
/// the test device's share is already saved.
/// </summary>
public sealed class KoppelFactory : IDisposable
{
    private readonly WebApplicationFactory<Program> _factory;
    private readonly bool _kestrel;
    private readonly object _session = new();
    private string? _cookie;

    public KoppelFactory(bool kestrel = false, Action<IServiceCollection>? services = null, IReadOnlyDictionary<string, string>? configuration = null, string role = "server", bool shareDongle = false, bool shareOnStartup = true, bool singlePort = false)
    {
        _kestrel = kestrel;
        Directory = Path.Combine(Path.GetTempPath(), "koppel-tests", Guid.NewGuid().ToString("N"));
        HttpPort = FreePort();
        HttpsPort = singlePort ? HttpPort : FreePort();

        // Seed settings so parallel hosts never fight over the default USB port.
        System.IO.Directory.CreateDirectory(Path.Combine(Directory, "config"));
        File.WriteAllText(ConfigFile("settings.json"), $$"""{ "role": "{{role}}", "shareOnStartup": {{(shareOnStartup ? "true" : "false")}} }""");
        if (shareDongle)
        {
            // What a person saved by sharing the device in the UI; the share is applied when the host starts.
            File.WriteAllText(ConfigFile("devices.json"), """{ "busIds": ["1-1.4"], "vidPids": { "1-1.4": "064f:2af9" } }""");
        }

        var factory = new WebApplicationFactory<Program>().WithWebHostBuilder(builder =>
        {
            builder.UseEnvironment("Testing");
            builder.UseSetting("Paths:Config", Path.Combine(Directory, "config"));
            builder.UseSetting("Paths:Keys", Path.Combine(Directory, "keys"));
            builder.UseSetting("Ports:Ui", HttpPort.ToString(System.Globalization.CultureInfo.InvariantCulture));
            builder.UseSetting("Ports:Network", HttpsPort.ToString(System.Globalization.CultureInfo.InvariantCulture));
            // The gateway is on by default on Windows; parallel hosts would fight over its port.
            builder.UseSetting("Gateway:Port", "0");
            // Test hosts never announce themselves on the real network.
            builder.UseSetting("Discovery:Enabled", "false");
            foreach (var (key, value) in configuration ?? new Dictionary<string, string>())
            {
                builder.UseSetting(key, value);
            }

            builder.ConfigureTestServices(s =>
            {
                // A test server has no listener port; give each request the one its scheme stands for.
                if (!kestrel)
                {
                    s.AddTransient<IStartupFilter>(_ => new TestListenerPorts(HttpPort, HttpsPort));
                }

                // Never the machine's own usbip-win2: a test that wants a driver brings a fake one.
                s.AddSingleton<IUsbIpDriver>(new UnsupportedUsb("No USB/IP driver in tests."));
                // Tests connect to the gateway from a user-mode process, which the Windows kernel-only check refuses.
                s.AddSingleton<Koppel.Usb.IGatewayPeerCheck>(new Koppel.Usb.AnyPeer());
                services?.Invoke(s);
            });
        });
        if (kestrel)
        {
            factory.UseKestrel();
            factory.StartServer();
        }

        _factory = factory;
    }

    public string Directory { get; }

    public int HttpPort { get; }

    public int HttpsPort { get; }

    public IServiceProvider Services => _factory.Services;

    public string ConfigFile(string name) => Path.Combine(Directory, "config", name);

    /// <summary>The password the first signed-in client sets.</summary>
    public const string Password = "correct horse battery";

    /// <summary>
    /// A client on the UI side that is signed in, sending the header the UI sends on every
    /// request. The first one sets the password with the setup code; later ones share its session.
    /// </summary>
    public HttpClient CreateUiClient()
    {
        var client = CreateAnonymousUiClient();
        client.DefaultRequestHeaders.Add("Cookie", SessionCookie());
        return client;
    }

    /// <summary>A signed-in client without the header, as another site's page would be if it could send the cookie.</summary>
    public HttpClient CreateSignedInPlainClient()
    {
        var client = CreatePlainClient();
        client.DefaultRequestHeaders.Add("Cookie", SessionCookie());
        return client;
    }

    /// <summary>A UI-side client with the header but no session, as a visitor who has not signed in.</summary>
    public HttpClient CreateAnonymousUiClient()
    {
        var client = CreatePlainClient();
        client.DefaultRequestHeaders.Add("X-Koppel-UI", "1");
        return client;
    }

    /// <summary>The setup code of a host that has no password yet.</summary>
    public string SetupCode => Services.GetRequiredService<Koppel.Login.UiAuthStore>().SetupCode ?? throw new InvalidOperationException("A password is already set.");

    private string SessionCookie()
    {
        lock (_session)
        {
            if (_cookie is null)
            {
                using var client = CreateAnonymousUiClient();
                // A test that set the password itself leaves nothing to set up: sign in instead.
                var passwordSet = Services.GetRequiredService<Koppel.Login.UiAuthStore>().SetupCode is null;
                using var response = (passwordSet
                    ? client.PostAsJsonAsync("/api/auth/login", new { password = Password })
                    : client.PostAsJsonAsync("/api/auth/setup", new { setupCode = SetupCode, password = Password })).GetAwaiter().GetResult();
                response.EnsureSuccessStatusCode();
                _cookie = response.Headers.GetValues("Set-Cookie").Single().Split(';')[0];
            }

            return _cookie;
        }
    }

    /// <summary>
    /// A UI-side client without the header, as another site's page would be. On a Kestrel host it connects
    /// to the real UI port and trusts exactly the server's own certificate.
    /// </summary>
    public HttpClient CreatePlainClient() => _kestrel
        ? new HttpClient(Koppel.Core.Security.ServerCertificate.CreatePinnedHandler(Usb.RemoteAccess.Fingerprint(this))) { BaseAddress = new Uri($"https://localhost:{HttpPort}") }
        : _factory.CreateClient();

    /// <summary>
    /// A client that arrives over HTTPS, as remote Koppel clients do. On a Kestrel host it connects to the
    /// real HTTPS port and accepts the server's self-signed certificate.
    /// </summary>
    public HttpClient CreateHttpsClient() => _kestrel
        ? new HttpClient(new HttpClientHandler { ServerCertificateCustomValidationCallback = (_, _, _, _) => true }) { BaseAddress = new Uri($"https://localhost:{HttpsPort}") }
        : _factory.CreateClient(new WebApplicationFactoryClientOptions { BaseAddress = new Uri("https://localhost") });

    /// <summary>The UI hub, signed in like <see cref="CreateUiClient"/>.</summary>
    public async Task<HubConnection> ConnectUiHubAsync(CancellationToken cancellationToken)
    {
        var connection = BuildUiHub(SessionCookie());
        await connection.StartAsync(cancellationToken);
        return connection;
    }

    /// <summary>The UI hub without a session. Starting it fails with 401 once a password is set.</summary>
    public HubConnection BuildAnonymousUiHub() => BuildUiHub(null);

    private HubConnection BuildUiHub(string? cookie)
    {
        void Headers(Microsoft.AspNetCore.Http.Connections.Client.HttpConnectionOptions options)
        {
            if (cookie is not null)
            {
                options.Headers["Cookie"] = cookie;
            }
        }

        return _kestrel
            ? new HubConnectionBuilder()
                .WithUrl($"https://localhost:{HttpPort}/api/hubs/ui", options =>
                {
                    TrustCertificate(options);
                    Headers(options);
                })
                .Build()
            : new HubConnectionBuilder()
                .WithUrl(new Uri(_factory.Server.BaseAddress, "/api/hubs/ui"), options =>
                {
                    options.HttpMessageHandlerFactory = _ => _factory.Server.CreateHandler();
                    options.Transports = Microsoft.AspNetCore.Http.Connections.HttpTransportType.LongPolling;
                    Headers(options);
                })
                .Build();
    }

    /// <summary>Lets a hub connection to this Kestrel host trust exactly the server's own certificate, over HTTP and WebSockets.</summary>
    public void TrustCertificate(Microsoft.AspNetCore.Http.Connections.Client.HttpConnectionOptions options)
    {
        options.HttpMessageHandlerFactory = _ => Koppel.Core.Security.ServerCertificate.CreatePinnedHandler(Usb.RemoteAccess.Fingerprint(this));
        options.WebSocketConfiguration = ws => ws.RemoteCertificateValidationCallback = (_, certificate, _, _) => Usb.RemoteAccess.Pinned(this, certificate);
    }

    public void Dispose()
    {
        _factory.Dispose();
        if (System.IO.Directory.Exists(Directory))
        {
            System.IO.Directory.Delete(Directory, recursive: true);
        }
    }

    // Ports are handed out once per process, so parallel hosts never get the same one even when the
    // first host has not bound it yet. The OS picks each one (port 0); another process can still take
    // it before the host binds, which a fixed port in Program cannot avoid.
    private static readonly HashSet<int> Taken = [];

    public static int FreePort()
    {
        lock (Taken)
        {
            while (true)
            {
                using var listener = new TcpListener(IPAddress.Loopback, 0);
                listener.Start();
                var port = ((IPEndPoint)listener.LocalEndpoint).Port;
                if (Taken.Add(port))
                {
                    return port;
                }
            }
        }
    }
}
