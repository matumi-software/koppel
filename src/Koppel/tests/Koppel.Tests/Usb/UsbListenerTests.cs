using System.Net;
using System.Net.Http.Json;
using System.Net.Sockets;
using System.Text;
using System.Text.Json;
using Koppel.Core.Usb;
using Microsoft.AspNetCore.SignalR.Client;
using Microsoft.Extensions.DependencyInjection;

namespace Koppel.Tests.Usb;

/// <summary>The USB port itself: who may wait on it, which role opens it, and what a port change does to live sessions.</summary>
public sealed class UsbListenerTests : IDisposable
{
    private readonly FakeKernel _kernel = new();
    private readonly KoppelFactory _server;

    public UsbListenerTests() => _server = new KoppelFactory(kestrel: true, services: s => s.AddSingleton<IUsbHost>(_kernel), shareDongle: true);

    public void Dispose() => _server.Dispose();

    [Fact]
    public async Task An_empty_token_is_denied()
    {
        var ct = TestContext.Current.CancellationToken;

        var (tls, answer) = await RemoteAccess.OpenUsbAsync(_server, "", ct);
        await using var session = tls;

        answer.Should().StartWith("DENIED");
    }

    [Fact]
    public async Task A_line_that_is_not_the_handshake_is_denied()
    {
        var ct = TestContext.Current.CancellationToken;
        using var tcp = new TcpClient();
        await tcp.ConnectAsync("localhost", _server.UsbPort, ct);
        await using var tls = new System.Net.Security.SslStream(tcp.GetStream(), false, (_, certificate, _, _) => RemoteAccess.Pinned(_server, certificate));
        await tls.AuthenticateAsClientAsync("localhost");

        await tls.WriteAsync(Encoding.ASCII.GetBytes("GET / HTTP/1.1\n"), ct);

        (await Koppel.Usb.UsbIpTlsServer.ReadLineAsync(tls, ct)).Should().StartWith("DENIED");
    }

    [Fact]
    public async Task Connections_that_never_finish_the_handshake_are_capped_per_address_and_time_out()
    {
        var ct = TestContext.Current.CancellationToken;
        var idle = new List<TcpClient>();
        try
        {
            for (var i = 0; i < 8; i++)
            {
                var tcp = new TcpClient();
                idle.Add(tcp);
                await tcp.ConnectAsync("localhost", _server.UsbPort, ct);
            }

            // The ninth from the same address is closed without a handshake.
            using var ninth = new TcpClient();
            await ninth.ConnectAsync("localhost", _server.UsbPort, ct);
            (await RemoteAccess.EndsWithinAsync(ninth.GetStream(), TimeSpan.FromSeconds(3), ct)).Should().BeTrue("a ninth waiting handshake from one address is refused");

            // The eight that wait are cut off at the pre-auth budget.
            (await RemoteAccess.EndsWithinAsync(idle[0].GetStream(), TimeSpan.FromSeconds(10), ct)).Should().BeTrue("a handshake that never finishes is closed");
        }
        finally
        {
            foreach (var tcp in idle)
            {
                tcp.Dispose();
            }
        }
    }

    [Fact]
    public async Task A_port_change_keeps_live_sessions_and_clients_learn_the_new_port()
    {
        var ct = TestContext.Current.CancellationToken;
        var (clientId, secret) = await RemoteAccess.CreateClientAsync(_server, "bench-2", ct);
        var token = await RemoteAccess.TokenAsync(_server, clientId, secret, ct);
        var (tls, answer) = await RemoteAccess.OpenUsbAsync(_server, token, ct);
        await using var session = tls;
        answer.Should().Be("OK");
        await RemoteAccess.ImportAsync(tls, "1-1.4", ct);
        var (hub, _, first) = await RemoteAccess.ConnectHubAsync(_server, token, ct);
        await using var link = hub;
        (await first.WaitAsync(TimeSpan.FromSeconds(10), ct)).GetProperty("usbPort").GetInt32().Should().Be(_server.UsbPort);
        var newPort = KoppelFactory.FreePort();
        var moved = new TaskCompletionSource<JsonElement>(TaskCreationOptions.RunContinuationsAsynchronously);
        hub.On<JsonElement>("DevicesChanged", devices =>
        {
            if (devices.GetProperty("usbPort").GetInt32() == newPort)
            {
                moved.TrySetResult(devices);
            }
        });

        using var ui = _server.CreateUiClient();
        (await ui.PatchAsJsonAsync("/api/settings", new { listenPort = newPort }, ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);

        await moved.Task.WaitAsync(TimeSpan.FromSeconds(10), ct);
        await UsbEndToEndTests.EchoesAsync(tls, ct);
        var (onNew, welcome) = await RemoteAccess.OpenUsbAsync(_server, token, ct, newPort);
        await using var fresh = onNew;
        welcome.Should().Be("OK");
        var refused = async () =>
        {
            using var old = new TcpClient();
            await old.ConnectAsync("localhost", _server.UsbPort, ct);
        };
        await refused.Should().ThrowAsync<SocketException>("the old port is closed");
    }

    [Theory]
    [InlineData("http")]
    [InlineData("https")]
    public async Task A_usb_port_equal_to_the_web_ports_is_rejected(string which)
    {
        var ct = TestContext.Current.CancellationToken;
        using var ui = _server.CreateUiClient();

        using var response = await ui.PatchAsJsonAsync("/api/settings", new { listenPort = which == "http" ? _server.HttpPort : _server.HttpsPort }, ct);

        response.StatusCode.Should().Be(HttpStatusCode.BadRequest);
        (await response.Content.ReadFromJsonAsync<JsonElement>(ct)).GetProperty("errors").TryGetProperty("listenPort", out _).Should().BeTrue();
        (await ui.GetFromJsonAsync<JsonElement>("/api/settings", ct)).GetProperty("listenPort").GetInt32().Should().Be(_server.UsbPort);
    }

    [Fact]
    public async Task A_usb_port_that_is_taken_is_reported_as_a_system_check()
    {
        var ct = TestContext.Current.CancellationToken;
        using var squatter = new TcpListener(IPAddress.IPv6Any, 0);
        squatter.Server.DualMode = true;
        squatter.Start();
        var taken = ((IPEndPoint)squatter.LocalEndpoint).Port;
        using var ui = _server.CreateUiClient();

        (await ui.PatchAsJsonAsync("/api/settings", new { listenPort = taken }, ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);

        await Eventually(async () =>
        {
            var checks = await ui.GetFromJsonAsync<JsonElement>("/api/system/checks", ct);
            var listen = checks.EnumerateArray().First(c => c.GetProperty("id").GetString() == "listen");
            return !listen.GetProperty("ok").GetBoolean() && listen.GetProperty("detail").GetString()!.Contains(taken.ToString(System.Globalization.CultureInfo.InvariantCulture), StringComparison.Ordinal);
        }, ct);
    }

    [Fact]
    public async Task Leaving_the_server_role_unshares_what_was_shared_and_closes_the_usb_port()
    {
        var ct = TestContext.Current.CancellationToken;
        _kernel.IsBound("1-1.4").Should().BeTrue();
        using var ui = _server.CreateUiClient();

        (await ui.PatchAsJsonAsync("/api/settings", new { role = "client" }, ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);

        await Eventually(() => Task.FromResult(!_kernel.IsBound("1-1.4")), ct);
        await Eventually(async () => !await CanConnectAsync(_server.UsbPort, ct), ct);

        (await ui.PatchAsJsonAsync("/api/settings", new { role = "server" }, ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);

        await Eventually(() => Task.FromResult(_kernel.IsBound("1-1.4")), ct);
        await Eventually(() => CanConnectAsync(_server.UsbPort, ct), ct);
    }

    [Fact]
    public async Task Leaving_the_server_role_ends_live_sessions()
    {
        var ct = TestContext.Current.CancellationToken;
        var (clientId, secret) = await RemoteAccess.CreateClientAsync(_server, "bench-2", ct);
        var token = await RemoteAccess.TokenAsync(_server, clientId, secret, ct);
        var (tls, answer) = await RemoteAccess.OpenUsbAsync(_server, token, ct);
        await using var session = tls;
        answer.Should().Be("OK");
        await RemoteAccess.ImportAsync(tls, "1-1.4", ct);

        using var ui = _server.CreateUiClient();
        (await ui.PatchAsJsonAsync("/api/settings", new { role = "client" }, ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);

        (await RemoteAccess.EndsWithinAsync(tls, TimeSpan.FromSeconds(10), ct)).Should().BeTrue();
    }

    private static async Task<bool> CanConnectAsync(int port, CancellationToken ct)
    {
        try
        {
            using var tcp = new TcpClient();
            await tcp.ConnectAsync("localhost", port, ct);
            return true;
        }
        catch (SocketException)
        {
            return false;
        }
    }

    private static async Task Eventually(Func<Task<bool>> condition, CancellationToken ct)
    {
        var deadline = DateTime.UtcNow.AddSeconds(15);
        while (!await condition())
        {
            if (DateTime.UtcNow > deadline)
            {
                throw new TimeoutException("The condition did not hold in time.");
            }

            await Task.Delay(100, ct);
        }
    }
}
