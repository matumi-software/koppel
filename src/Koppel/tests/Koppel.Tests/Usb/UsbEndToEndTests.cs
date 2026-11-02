using System.Net.Http.Json;
using System.Net.Security;
using System.Net.Sockets;
using System.Text;
using System.Text.Json;
using Koppel.Core.Security;
using Koppel.Core.Usb;
using Microsoft.AspNetCore.SignalR.Client;
using Microsoft.Extensions.DependencyInjection;

namespace Koppel.Tests.Usb;

/// <summary>
/// Two real Koppel hosts on Kestrel, a server and a client, with a fake kernel on each side. Covers
/// everything except the kernel itself: OAuth, pinning, the remote hub, the TLS handshake, USB/IP
/// import and the encrypted relay.
/// </summary>
public sealed class UsbEndToEndTests : IDisposable
{
    private readonly FakeKernel _serverKernel = new();
    private readonly FakeKernel _clientKernel = new();
    private readonly KoppelFactory _server;
    private readonly KoppelFactory _client;

    public UsbEndToEndTests()
    {
        _server = new KoppelFactory(kestrel: true, services: s => s.AddSingleton<IUsbHost>(_serverKernel));
        _client = new KoppelFactory(kestrel: true, services: s => s.AddSingleton<IUsbAttacher>(_clientKernel));
    }

    public void Dispose()
    {
        _client.Dispose();
        _server.Dispose();
    }

    [Fact]
    public async Task Client_attaches_a_shared_device_and_usb_traffic_round_trips_through_tls()
    {
        var ct = TestContext.Current.CancellationToken;
        await using var clientHub = await _client.ConnectUiHubAsync(ct);
        var linked = Waiter(clientHub, "ClientDevicesChanged", d =>
            d.GetProperty("link").GetString() == "connected" && d.GetProperty("devices").GetArrayLength() == 1);
        var attached = Waiter(clientHub, "ClientDevicesChanged", d =>
            d.GetProperty("devices").EnumerateArray().Any(x => x.GetProperty("attachedHere").GetBoolean()));

        await ConnectClientToServerAsync(ct);
        var devices = await WithLastAsync(linked, clientHub, ct);
        devices.GetProperty("devices")[0].GetProperty("device").GetProperty("vidPid").GetString().Should().Be("064f:2af9");

        using var ui = _client.CreateUiClient();
        using var attach = await ui.PostAsync("/api/client/devices/1-1.4/attach", null, ct);
        attach.StatusCode.Should().Be(System.Net.HttpStatusCode.Accepted);
        await attached.WaitAsync(TimeSpan.FromSeconds(20), ct);

        // The test plays the client's USB driver: bytes go out through TLS and the fake server kernel echoes them.
        var socket = await _clientKernel.AttachedSocket.Task.WaitAsync(TimeSpan.FromSeconds(5), ct);
        var urb = Encoding.ASCII.GetBytes("CMD_SUBMIT fake urb");
        await socket.SendAsync(urb, ct);
        var echoed = new byte[urb.Length];
        var total = 0;
        while (total < echoed.Length)
        {
            total += await socket.ReceiveAsync(echoed.AsMemory(total), ct);
        }

        echoed.Should().Equal(urb);
    }

    [Fact]
    public async Task Usb_port_refuses_a_connection_without_a_valid_token()
    {
        var ct = TestContext.Current.CancellationToken;
        using var server = _server.CreateUiClient();
        var fingerprint = (await server.GetFromJsonAsync<JsonElement>("/api/server/identity", ct)).GetProperty("fingerprint").GetString();
        using var tcp = new TcpClient();
        await tcp.ConnectAsync("localhost", _server.UsbPort, ct);
        await using var tls = new SslStream(tcp.GetStream(), false, (_, certificate, _, _) =>
            ServerCertificate.Matches(certificate as System.Security.Cryptography.X509Certificates.X509Certificate2, fingerprint));
        await tls.AuthenticateAsClientAsync("localhost");

        await tls.WriteAsync(Encoding.ASCII.GetBytes("KOPPEL/1 not-a-token\n"), ct);
        var reader = new StreamReader(tls);
        var answer = await reader.ReadLineAsync(ct);

        answer.Should().StartWith("DENIED");
    }

    private async Task ConnectClientToServerAsync(CancellationToken ct)
    {
        using var server = _server.CreateUiClient();
        var identity = await server.GetFromJsonAsync<JsonElement>("/api/server/identity", ct);
        using var created = await server.PostAsJsonAsync("/api/server/clients", new { name = "test-client" }, ct);
        var credentials = await created.Content.ReadFromJsonAsync<JsonElement>(ct);

        using var client = _client.CreateUiClient();
        (await client.PatchAsJsonAsync("/api/settings", new { role = "client" }, ct)).EnsureSuccessStatusCode();
        (await client.PutAsJsonAsync("/api/client/connection", new
        {
            serverUrl = $"https://localhost:{_server.HttpsPort}",
            serverFingerprint = identity.GetProperty("fingerprint").GetString(),
            clientId = credentials.GetProperty("clientId").GetString(),
            clientSecret = credentials.GetProperty("clientSecret").GetString(),
        }, ct)).EnsureSuccessStatusCode();
    }

    private static async Task<JsonElement> WithLastAsync(Task<JsonElement> waiter, HubConnection hub, CancellationToken ct)
    {
        var last = "nothing";
        hub.On<JsonElement>("ClientDevicesChanged", value => last = value.GetRawText());
        try
        {
            return await waiter.WaitAsync(TimeSpan.FromSeconds(20), ct);
        }
        catch (TimeoutException)
        {
            throw new TimeoutException($"Last ClientDevicesChanged: {last}");
        }
    }

    private static Task<JsonElement> Waiter(HubConnection hub, string method, Func<JsonElement, bool> until)
    {
        var done = new TaskCompletionSource<JsonElement>(TaskCreationOptions.RunContinuationsAsynchronously);
        hub.On<JsonElement>(method, value =>
        {
            if (until(value))
            {
                done.TrySetResult(value);
            }
        });
        return done.Task;
    }
}
