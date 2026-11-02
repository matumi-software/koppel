using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Koppel.Settings;
using Microsoft.AspNetCore.Hosting;
using Microsoft.AspNetCore.Mvc.Testing;
using Microsoft.AspNetCore.SignalR.Client;
using Microsoft.AspNetCore.TestHost;

namespace Koppel.Tests.Settings;

/// <summary>
/// The product rule end to end: PATCH answers 202 with no body, and the confirmed settings
/// arrive over SignalR only after they are on disk.
/// </summary>
public sealed class SettingsConfirmFlowTests : IDisposable
{
    private readonly string _directory = Path.Combine(Path.GetTempPath(), "koppel-tests", Guid.NewGuid().ToString("N"));
    private readonly WebApplicationFactory<Program> _factory;

    public SettingsConfirmFlowTests()
    {
        _factory = new WebApplicationFactory<Program>().WithWebHostBuilder(builder =>
        {
            builder.UseEnvironment("Testing");
            builder.UseSetting("Paths:Config", _directory);
            builder.UseSetting("Paths:Keys", Path.Combine(_directory, "keys"));
        });
    }

    public void Dispose()
    {
        _factory.Dispose();
        if (Directory.Exists(_directory))
        {
            Directory.Delete(_directory, recursive: true);
        }
    }

    [Fact]
    public async Task Patch_is_confirmed_over_SignalR_after_the_file_is_written()
    {
        var ct = TestContext.Current.CancellationToken;
        using var client = _factory.CreateClient();
        await using var hub = await ConnectAsync(ct);
        var confirmed = new TaskCompletionSource<JsonElement>(TaskCreationOptions.RunContinuationsAsynchronously);
        hub.On<JsonElement>("SettingsChanged", settings => confirmed.TrySetResult(settings));

        using var response = await client.PatchAsJsonAsync("/api/settings", new { shareOnStartup = false }, ct);

        response.StatusCode.Should().Be(HttpStatusCode.Accepted);
        (await response.Content.ReadAsStringAsync(ct)).Should().BeEmpty();
        var pushed = await confirmed.Task.WaitAsync(TimeSpan.FromSeconds(5), ct);
        pushed.GetProperty("shareOnStartup").GetBoolean().Should().BeFalse();
        var onDisk = JsonSerializer.Deserialize<KoppelSettings>(
            await File.ReadAllTextAsync(Path.Combine(_directory, "settings.json"), ct), KoppelJson.Files)!;
        onDisk.ShareOnStartup.Should().BeFalse();
    }

    [Fact]
    public async Task Patch_with_a_bad_port_is_rejected_and_nothing_is_pushed()
    {
        var ct = TestContext.Current.CancellationToken;
        using var client = _factory.CreateClient();
        await using var hub = await ConnectAsync(ct);
        var pushes = 0;
        hub.On<JsonElement>("SettingsChanged", _ => Interlocked.Increment(ref pushes));

        using var response = await client.PatchAsJsonAsync("/api/settings", new { listenPort = 70000 }, ct);

        response.StatusCode.Should().Be(HttpStatusCode.BadRequest);
        await Task.Delay(300, ct);
        pushes.Should().Be(0);
        (await client.GetFromJsonAsync<JsonElement>("/api/settings", ct)).GetProperty("listenPort").GetInt32().Should().Be(3240);
    }

    private async Task<HubConnection> ConnectAsync(CancellationToken cancellationToken)
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
}
