using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Koppel.Core.Usb;
using Microsoft.AspNetCore.SignalR.Client;
using Microsoft.Extensions.DependencyInjection;

namespace Koppel.Tests.Usb;

/// <summary>
/// A revoked client, or one whose secret was rotated, loses what it is doing now: its USB relay and its
/// hub link end, and a token from before a rotation no longer works anywhere.
/// </summary>
public sealed class RevocationTests : IDisposable
{
    private readonly FakeKernel _kernel = new();
    private readonly KoppelFactory _server;

    public RevocationTests() => _server = new KoppelFactory(kestrel: true, services: s => s.AddSingleton<IUsbHost>(_kernel), shareDongle: true);

    public void Dispose() => _server.Dispose();

    [Fact]
    public async Task Revoking_a_client_ends_its_usb_relay_and_its_hub_connection()
    {
        var ct = TestContext.Current.CancellationToken;
        var (clientId, secret) = await RemoteAccess.CreateClientAsync(_server, "bench-2", ct);
        var token = await RemoteAccess.TokenAsync(_server, clientId, secret, ct);
        var (tls, answer) = await RemoteAccess.OpenUsbAsync(_server, token, ct);
        await using var session = tls;
        answer.Should().Be("OK");
        await RemoteAccess.ImportAsync(tls, "1-1.4", ct);
        await UsbEndToEndTests.EchoesAsync(tls, ct);
        var (hub, closed, _) = await RemoteAccess.ConnectHubAsync(_server, token, ct);
        await using var link = hub;

        using var ui = _server.CreateUiClient();
        using var revoke = await ui.DeleteAsync($"/api/server/clients/{clientId}", ct);
        revoke.StatusCode.Should().Be(HttpStatusCode.Accepted);

        (await RemoteAccess.EndsWithinAsync(tls, TimeSpan.FromSeconds(10), ct)).Should().BeTrue("the relay ends when its client is revoked");
        await closed.WaitAsync(TimeSpan.FromSeconds(10), ct);
    }

    [Fact]
    public async Task Rotating_the_secret_ends_live_sessions_and_the_old_token_fails_everywhere()
    {
        var ct = TestContext.Current.CancellationToken;
        var (clientId, secret) = await RemoteAccess.CreateClientAsync(_server, "bench-2", ct);
        var oldToken = await RemoteAccess.TokenAsync(_server, clientId, secret, ct);
        (await RemoteAccess.HelloAsync(_server, oldToken, ct)).Should().Be(HttpStatusCode.OK);
        var (tls, answer) = await RemoteAccess.OpenUsbAsync(_server, oldToken, ct);
        await using var session = tls;
        answer.Should().Be("OK");
        await RemoteAccess.ImportAsync(tls, "1-1.4", ct);
        var (hub, closed, _) = await RemoteAccess.ConnectHubAsync(_server, oldToken, ct);
        await using var link = hub;

        using var ui = _server.CreateUiClient();
        using var rotate = await ui.PostAsync($"/api/server/clients/{clientId}/rotate", null, ct);
        var newSecret = (await rotate.Content.ReadFromJsonAsync<JsonElement>(ct)).GetProperty("clientSecret").GetString()!;

        (await RemoteAccess.EndsWithinAsync(tls, TimeSpan.FromSeconds(10), ct)).Should().BeTrue();
        await closed.WaitAsync(TimeSpan.FromSeconds(10), ct);
        (await RemoteAccess.HelloAsync(_server, oldToken, ct)).Should().Be(HttpStatusCode.Unauthorized);
        var (again, refused) = await RemoteAccess.OpenUsbAsync(_server, oldToken, ct);
        await using var denied = again;
        refused.Should().StartWith("DENIED");
        (await RemoteAccess.HubRefusesAsync(_server, oldToken, ct)).Should().BeTrue();

        var newToken = await RemoteAccess.TokenAsync(_server, clientId, newSecret, ct);
        (await RemoteAccess.HelloAsync(_server, newToken, ct)).Should().Be(HttpStatusCode.OK);
        var (fresh, welcome) = await RemoteAccess.OpenUsbAsync(_server, newToken, ct);
        await using var current = fresh;
        welcome.Should().Be("OK");
    }

    [Fact]
    public async Task Other_clients_sessions_survive_a_revoke()
    {
        var ct = TestContext.Current.CancellationToken;
        var (goneId, _) = await RemoteAccess.CreateClientAsync(_server, "gone", ct);
        var (stayId, staySecret) = await RemoteAccess.CreateClientAsync(_server, "stays", ct);
        var token = await RemoteAccess.TokenAsync(_server, stayId, staySecret, ct);
        var (tls, answer) = await RemoteAccess.OpenUsbAsync(_server, token, ct);
        await using var session = tls;
        answer.Should().Be("OK");
        await RemoteAccess.ImportAsync(tls, "1-1.4", ct);

        using var ui = _server.CreateUiClient();
        (await ui.DeleteAsync($"/api/server/clients/{goneId}", ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);

        await UsbEndToEndTests.EchoesAsync(tls, ct);
    }

    [Fact]
    public async Task Remote_view_hides_who_holds_a_device_and_the_serial()
    {
        var ct = TestContext.Current.CancellationToken;
        var (idA, secretA) = await RemoteAccess.CreateClientAsync(_server, "a", ct);
        var (idB, secretB) = await RemoteAccess.CreateClientAsync(_server, "b", ct);
        var tokenA = await RemoteAccess.TokenAsync(_server, idA, secretA, ct);
        var tokenB = await RemoteAccess.TokenAsync(_server, idB, secretB, ct);
        var (hub, _, first) = await RemoteAccess.ConnectHubAsync(_server, tokenB, ct);
        await using var link = hub;
        var taken = new TaskCompletionSource<JsonElement>(TaskCreationOptions.RunContinuationsAsynchronously);
        hub.On<JsonElement>("DevicesChanged", devices =>
        {
            if (devices.GetProperty("devices").EnumerateArray().Any(d => d.GetProperty("state").GetString() == "attached"))
            {
                taken.TrySetResult(devices);
            }
        });
        (await first.WaitAsync(TimeSpan.FromSeconds(10), ct)).GetProperty("devices")[0].GetProperty("serial").ValueKind.Should().Be(JsonValueKind.Null);

        var (tls, answer) = await RemoteAccess.OpenUsbAsync(_server, tokenA, ct);
        await using var session = tls;
        answer.Should().Be("OK");
        await RemoteAccess.ImportAsync(tls, "1-1.4", ct);
        var seen = (await taken.Task.WaitAsync(TimeSpan.FromSeconds(10), ct)).GetProperty("devices")[0];

        seen.GetProperty("attachedTo").ValueKind.Should().Be(JsonValueKind.Null);
        seen.GetProperty("attachedSince").ValueKind.Should().Be(JsonValueKind.Null);
        seen.GetProperty("serial").ValueKind.Should().Be(JsonValueKind.Null);
        seen.GetProperty("driver").ValueKind.Should().Be(JsonValueKind.Null);
        seen.GetProperty("fault").ValueKind.Should().Be(JsonValueKind.Null);
        seen.GetProperty("vidPid").GetString().Should().Be("064f:2af9");
        seen.GetRawText().Should().NotContain(idA);
    }
}

