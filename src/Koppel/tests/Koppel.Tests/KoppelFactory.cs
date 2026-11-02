using Microsoft.AspNetCore.Hosting;
using Microsoft.AspNetCore.Mvc.Testing;
using Microsoft.AspNetCore.SignalR.Client;
using Microsoft.AspNetCore.TestHost;

namespace Koppel.Tests;

/// <summary>A Koppel host on a test server with its own temporary config and key directories.</summary>
public sealed class KoppelFactory : IDisposable
{
    private readonly WebApplicationFactory<Program> _factory;

    public KoppelFactory()
    {
        Directory = Path.Combine(Path.GetTempPath(), "koppel-tests", Guid.NewGuid().ToString("N"));
        _factory = new WebApplicationFactory<Program>().WithWebHostBuilder(builder =>
        {
            builder.UseEnvironment("Testing");
            builder.UseSetting("Paths:Config", Path.Combine(Directory, "config"));
            builder.UseSetting("Paths:Keys", Path.Combine(Directory, "keys"));
        });
    }

    public string Directory { get; }

    public string ConfigFile(string name) => Path.Combine(Directory, "config", name);

    /// <summary>A client on the UI side (plain HTTP).</summary>
    public HttpClient CreateUiClient() => _factory.CreateClient();

    /// <summary>A client that looks like it arrived over HTTPS, as remote Koppel clients do.</summary>
    public HttpClient CreateHttpsClient() =>
        _factory.CreateClient(new WebApplicationFactoryClientOptions { BaseAddress = new Uri("https://localhost") });

    public async Task<HubConnection> ConnectUiHubAsync(CancellationToken cancellationToken)
    {
        var server = _factory.Server;
        var connection = new HubConnectionBuilder()
            .WithUrl(new Uri(server.BaseAddress, "/api/hubs/ui"), options =>
            {
                options.HttpMessageHandlerFactory = _ => server.CreateHandler();
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
}
