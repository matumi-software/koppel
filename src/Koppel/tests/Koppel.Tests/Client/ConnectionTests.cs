using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Koppel.Tests.Usb;
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

    [Fact]
    public async Task An_empty_secret_keeps_the_stored_one_only_while_the_server_is_unchanged()
    {
        var ct = TestContext.Current.CancellationToken;
        await using var hub = await _koppel.ConnectUiHubAsync(ct);
        using var ui = _koppel.CreateUiClient();
        await SaveAsync(ui, "https://lab-rack-3:5241", Fingerprint, "client-one", "first-secret", ct);
        var changed = Link.Waiter(hub, "ConnectionChanged", c => c.GetProperty("clientId").GetString() == "client-two");

        using var renamed = await SaveAsync(ui, "https://lab-rack-3:5241", Fingerprint, "client-two", "", ct);

        renamed.StatusCode.Should().Be(HttpStatusCode.Accepted);
        var view = await changed.WaitAsync(TimeSpan.FromSeconds(10), ct);
        view.GetProperty("hasSecret").GetBoolean().Should().BeTrue("same server: the secret stays");
    }

    [Theory]
    [InlineData("https://other-host:5241", Fingerprint, "serverUrl")]
    [InlineData("https://lab-rack-3:5241", "00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF", "serverFingerprint")]
    public async Task An_empty_secret_is_refused_when_the_server_address_or_fingerprint_changes(string url, string fingerprint, string changed)
    {
        var ct = TestContext.Current.CancellationToken;
        using var ui = _koppel.CreateUiClient();
        await SaveAsync(ui, "https://lab-rack-3:5241", Fingerprint, "client-one", "first-secret", ct);

        using var refused = await SaveAsync(ui, url, fingerprint, "client-one", "", ct);

        refused.StatusCode.Should().Be(HttpStatusCode.BadRequest, changed);
        (await refused.Content.ReadAsStringAsync(ct)).Should().Contain("clientSecret");
        var stored = await ui.GetFromJsonAsync<JsonElement>("/api/client/connection", ct);
        stored.GetProperty("serverUrl").GetString().Should().Be("https://lab-rack-3:5241", "nothing was saved");
        stored.GetProperty("hasSecret").GetBoolean().Should().BeTrue();
    }

    [Fact]
    public async Task A_first_save_without_a_secret_is_refused()
    {
        using var ui = _koppel.CreateUiClient();

        using var refused = await SaveAsync(ui, "https://lab-rack-3:5241", Fingerprint, "client-one", "", TestContext.Current.CancellationToken);

        refused.StatusCode.Should().Be(HttpStatusCode.BadRequest);
    }

    [Fact]
    public async Task A_new_secret_is_accepted_for_another_server()
    {
        var ct = TestContext.Current.CancellationToken;
        using var ui = _koppel.CreateUiClient();
        await SaveAsync(ui, "https://lab-rack-3:5241", Fingerprint, "client-one", "first-secret", ct);

        using var moved = await SaveAsync(ui, "https://other-host:5241", Fingerprint, "client-two", "second-secret", ct);

        moved.StatusCode.Should().Be(HttpStatusCode.Accepted);
    }

    private static Task<HttpResponseMessage> SaveAsync(HttpClient ui, string url, string fingerprint, string clientId, string secret, CancellationToken ct) =>
        ui.PutAsJsonAsync("/api/client/connection", new { serverUrl = url, serverFingerprint = fingerprint, clientId, clientSecret = secret }, ct);
}
