using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Microsoft.AspNetCore.SignalR.Client;

namespace Koppel.Tests.Client;

/// <summary>The client role's stored connection: confirmed over SignalR, encrypted on disk, secret never sent back.</summary>
public sealed class ConnectionTests : IDisposable
{
    private const string Fingerprint = "3F:9A:1C:2B:7D:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA";
    private readonly KoppelFactory _koppel = new();

    public void Dispose() => _koppel.Dispose();

    [Fact]
    public async Task Saving_the_connection_is_confirmed_without_the_secret()
    {
        var ct = TestContext.Current.CancellationToken;
        await using var hub = await _koppel.ConnectUiHubAsync(ct);
        var pushed = new TaskCompletionSource<JsonElement>(TaskCreationOptions.RunContinuationsAsynchronously);
        hub.On<JsonElement>("ConnectionChanged", connection => pushed.TrySetResult(connection));
        using var ui = _koppel.CreateUiClient();

        using var response = await ui.PutAsJsonAsync("/api/client/connection", new
        {
            serverUrl = "https://lab-rack-3:5241/some/path",
            serverFingerprint = Fingerprint.Replace(":", "", StringComparison.Ordinal).ToLowerInvariant(),
            clientId = "client-0a1b2c3d4e",
            clientSecret = "s3cret-value-that-must-stay-secret",
        }, ct);

        response.StatusCode.Should().Be(HttpStatusCode.Accepted);
        var connection = await pushed.Task.WaitAsync(TimeSpan.FromSeconds(5), ct);
        connection.GetProperty("serverUrl").GetString().Should().Be("https://lab-rack-3:5241");
        connection.GetProperty("serverFingerprint").GetString().Should().Be(Fingerprint);
        connection.GetProperty("hasSecret").GetBoolean().Should().BeTrue();
        connection.GetRawText().Should().NotContain("s3cret");
        var onDisk = await File.ReadAllTextAsync(_koppel.ConfigFile("connection.json"), ct);
        onDisk.Should().NotContain("s3cret").And.NotContain("lab-rack-3");
    }

    [Fact]
    public async Task Plain_http_server_address_is_rejected()
    {
        var ct = TestContext.Current.CancellationToken;
        using var ui = _koppel.CreateUiClient();

        using var response = await ui.PutAsJsonAsync("/api/client/connection", new
        {
            serverUrl = "http://lab-rack-3:5240",
            serverFingerprint = Fingerprint,
            clientId = "client-0a1b2c3d4e",
            clientSecret = "x",
        }, ct);

        response.StatusCode.Should().Be(HttpStatusCode.BadRequest);
        (await response.Content.ReadAsStringAsync(ct)).Should().Contain("serverUrl");
    }
}
