using System.Net;
using System.Net.Sockets;
using Microsoft.AspNetCore.Hosting;
using Microsoft.AspNetCore.Mvc.Testing;
using Microsoft.AspNetCore.SignalR.Client;
using Microsoft.AspNetCore.TestHost;
using Microsoft.Extensions.DependencyInjection;

namespace Koppel.Tests;

/// <summary>
/// A Koppel host with its own temporary config and key directories. By default it runs on a test
/// server. With <c>kestrel: true</c> it listens on real free ports, so two hosts can talk as server
/// and client, and the TLS USB port can be reached.
/// </summary>
public sealed class KoppelFactory : IDisposable
{
    private readonly WebApplicationFactory<Program> _factory;
    private readonly bool _kestrel;

    public KoppelFactory(bool kestrel = false, Action<IServiceCollection>? services = null)
    {
        _kestrel = kestrel;
        Directory = Path.Combine(Path.GetTempPath(), "koppel-tests", Guid.NewGuid().ToString("N"));
        HttpPort = FreePort();
        HttpsPort = FreePort();
        UsbPort = FreePort();

        // Seed settings so parallel hosts never fight over the default USB port.
        System.IO.Directory.CreateDirectory(Path.Combine(Directory, "config"));
        File.WriteAllText(ConfigFile("settings.json"), $$"""{ "role": "server", "shareOnStartup": true, "listenPort": {{UsbPort}} }""");

        var factory = new WebApplicationFactory<Program>().WithWebHostBuilder(builder =>
        {
            builder.UseEnvironment("Testing");
            builder.UseSetting("Paths:Config", Path.Combine(Directory, "config"));
            builder.UseSetting("Paths:Keys", Path.Combine(Directory, "keys"));
            builder.UseSetting("Ports:Http", HttpPort.ToString(System.Globalization.CultureInfo.InvariantCulture));
            builder.UseSetting("Ports:Https", HttpsPort.ToString(System.Globalization.CultureInfo.InvariantCulture));
            if (services is not null)
            {
                builder.ConfigureTestServices(services);
            }
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

    public int UsbPort { get; }

    public IServiceProvider Services => _factory.Services;

    public string ConfigFile(string name) => Path.Combine(Directory, "config", name);

    /// <summary>A client on the UI side (plain HTTP).</summary>
    public HttpClient CreateUiClient() => _kestrel
        ? new HttpClient { BaseAddress = new Uri($"http://localhost:{HttpPort}") }
        : _factory.CreateClient();

    /// <summary>A client that looks like it arrived over HTTPS, as remote Koppel clients do.</summary>
    public HttpClient CreateHttpsClient() =>
        _factory.CreateClient(new WebApplicationFactoryClientOptions { BaseAddress = new Uri("https://localhost") });

    public async Task<HubConnection> ConnectUiHubAsync(CancellationToken cancellationToken)
    {
        var connection = _kestrel
            ? new HubConnectionBuilder().WithUrl($"http://localhost:{HttpPort}/api/hubs/ui").Build()
            : new HubConnectionBuilder()
                .WithUrl(new Uri(_factory.Server.BaseAddress, "/api/hubs/ui"), options =>
                {
                    options.HttpMessageHandlerFactory = _ => _factory.Server.CreateHandler();
                    options.Transports = Microsoft.AspNetCore.Http.Connections.HttpTransportType.LongPolling;
                })
                .Build();
        await connection.StartAsync(cancellationToken);
        return connection;
    }

    public void Dispose()
    {
        _factory.Dispose();
        if (System.IO.Directory.Exists(Directory))
        {
            System.IO.Directory.Delete(Directory, recursive: true);
        }
    }

    private static int FreePort()
    {
        using var listener = new TcpListener(IPAddress.Loopback, 0);
        listener.Start();
        return ((IPEndPoint)listener.LocalEndpoint).Port;
    }
}
