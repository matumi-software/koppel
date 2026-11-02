using System.Net;
using System.Net.Http.Json;
using System.Net.Security;
using System.Net.Sockets;
using System.Text;
using System.Text.Json;
using Koppel.Core.Security;
using Koppel.Core.Usb;
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
    private readonly int _gatewayPort = KoppelFactory.FreePort();
    private readonly KoppelFactory _server;
    private readonly KoppelFactory _client;

    public UsbEndToEndTests()
    {
        _server = new KoppelFactory(kestrel: true, services: s => s.AddSingleton<IUsbHost>(_serverKernel), shareDongle: true);
        _client = new KoppelFactory(
            kestrel: true,
            services: s => s.AddSingleton<IUsbAttacher>(_clientKernel),
            configuration: new Dictionary<string, string> { ["Gateway:Port"] = _gatewayPort.ToString(System.Globalization.CultureInfo.InvariantCulture) });
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
        var linked = Link.Waiter(clientHub, "ClientDevicesChanged", d =>
            d.GetProperty("link").GetString() == "connected" && d.GetProperty("devices").GetArrayLength() == 1);
        var attached = Link.Waiter(clientHub, "ClientDevicesChanged", d =>
            d.GetProperty("devices").EnumerateArray().Any(x => x.GetProperty("attachedHere").GetBoolean()));

        await Link.ConnectAsync(_server, _client, ct);
        var devices = await Link.WithLastAsync(linked, clientHub, ct);
        devices.GetProperty("devices")[0].GetProperty("device").GetProperty("vidPid").GetString().Should().Be("064f:2af9");

        using var ui = _client.CreateUiClient();
        using var attach = await ui.PostAsync("/api/client/devices/1-1.4/attach", null, ct);
        attach.StatusCode.Should().Be(HttpStatusCode.Accepted);
        await attached.WaitAsync(TimeSpan.FromSeconds(20), ct);

        // The test plays the client's USB driver: bytes go out through TLS and the fake server kernel echoes them.
        var socket = await _clientKernel.AttachedSocket.Task.WaitAsync(TimeSpan.FromSeconds(5), ct);
        await using var driver = new NetworkStream(socket, ownsSocket: false);
        await EchoesAsync(driver, ct);
    }

    [Fact]
    public async Task Client_attaches_a_device_again_after_its_link_drops()
    {
        var ct = TestContext.Current.CancellationToken;
        await AttachAsync(ct);
        var first = await _clientKernel.AttachedSocket.Task.WaitAsync(TimeSpan.FromSeconds(5), ct);

        // The link to the server drops, as when the server restarts.
        first.Dispose();

        var again = await _clientKernel.Reattached.Task.WaitAsync(TimeSpan.FromSeconds(20), ct);
        again.Should().NotBeSameAs(first);
    }

    [Fact]
    public async Task Client_remembers_what_it_attached_until_it_is_detached()
    {
        var ct = TestContext.Current.CancellationToken;
        await AttachAsync(ct);
        File.ReadAllText(_client.ConfigFile("attached.json")).Should().Contain("1-1.4");

        using var ui = _client.CreateUiClient();
        using var detach = await ui.PostAsync("/api/client/devices/1-1.4/detach", null, ct);

        detach.StatusCode.Should().Be(HttpStatusCode.Accepted);
        File.ReadAllText(_client.ConfigFile("attached.json")).Should().NotContain("1-1.4");
    }

    [Fact]
    public async Task Server_resets_the_device_before_a_client_gets_it()
    {
        var ct = TestContext.Current.CancellationToken;
        await using var clientHub = await _client.ConnectUiHubAsync(ct);
        var linked = Link.Waiter(clientHub, "ClientDevicesChanged", d =>
            d.GetProperty("link").GetString() == "connected" && d.GetProperty("devices").GetArrayLength() == 1);
        var attached = Link.Waiter(clientHub, "ClientDevicesChanged", d =>
            d.GetProperty("devices").EnumerateArray().Any(x => x.GetProperty("attachedHere").GetBoolean()));
        await Link.ConnectAsync(_server, _client, ct);
        await Link.WithLastAsync(linked, clientHub, ct);

        using var ui = _client.CreateUiClient();
        using var attach = await ui.PostAsync("/api/client/devices/1-1.4/attach", null, ct);
        await attached.WaitAsync(TimeSpan.FromSeconds(20), ct);

        // Drivers on the server may have changed the device before it was shared; a reset undoes that.
        _serverKernel.ResetBeforeExport.Should().BeTrue();
    }

    [Fact]
    public async Task Gateway_lets_a_usbip_driver_on_the_client_machine_list_and_attach_through_tls()
    {
        var ct = TestContext.Current.CancellationToken;
        await using var clientHub = await _client.ConnectUiHubAsync(ct);
        var linked = Link.Waiter(clientHub, "ClientDevicesChanged", d =>
            d.GetProperty("link").GetString() == "connected" && d.GetProperty("devices").GetArrayLength() == 1 && d.GetProperty("gateway").GetBoolean());
        var attached = Link.Waiter(clientHub, "ClientDevicesChanged", d =>
            d.GetProperty("devices").EnumerateArray().Any(x => x.GetProperty("attachedVia").ValueKind == JsonValueKind.String));

        await Link.ConnectAsync(_server, _client, ct);
        await Link.WithLastAsync(linked, clientHub, ct);

        // The test plays usbip-win2: "usbip list -r 127.0.0.1", then "usbip attach -r 127.0.0.1 -b 1-1.4".
        using (var list = new TcpClient())
        {
            await list.ConnectAsync(IPAddress.Loopback, _gatewayPort, ct);
            var stream = list.GetStream();
            await stream.WriteAsync(UsbIpProtocol.DevListRequest(), ct);
            var header = await UsbIpProtocol.ReadHeaderAsync(stream, ct);
            header.Should().Be(new UsbIpProtocol.OpHeader(UsbIpProtocol.Version, UsbIpProtocol.OpRepDevList, UsbIpProtocol.StatusOk));
            var listed = UsbIpProtocol.ReadDevList(stream);
            listed.Should().ContainSingle().Which.BusId.Should().Be("1-1.4");
            listed[0].Interfaces.Should().Equal(FakeKernel.Dongle.Interfaces);
        }

        using var import = new TcpClient();
        await import.ConnectAsync(IPAddress.Loopback, _gatewayPort, ct);
        var usb = import.GetStream();
        await usb.WriteAsync(UsbIpProtocol.ImportRequest("1-1.4"), ct);
        var reply = await UsbIpProtocol.ReadHeaderAsync(usb, ct);
        reply.Status.Should().Be(UsbIpProtocol.StatusOk);
        var device = await UsbIpProtocol.ReadImportedDeviceAsync(usb, ct);
        device.VidPid.Should().Be("064f:2af9");
        device.DevId.Should().Be(FakeKernel.Dongle.DevId);

        var pushed = await Link.WithLastAsync(attached, clientHub, ct);
        var entry = pushed.GetProperty("devices")[0];
        entry.GetProperty("attachedHere").GetBoolean().Should().BeTrue();
        entry.GetProperty("port").ValueKind.Should().Be(JsonValueKind.Null);
        await EchoesAsync(usb, ct);
    }

    [Fact]
    public async Task With_a_driver_attach_and_detach_drive_it_through_the_gateway()
    {
        var ct = TestContext.Current.CancellationToken;
        using var driver = new FakeUsbIpDriver();
        using var client = new KoppelFactory(
            kestrel: true,
            services: s => s.AddSingleton<IUsbIpDriver>(driver),
            configuration: new Dictionary<string, string> { ["Gateway:Port"] = KoppelFactory.FreePort().ToString(System.Globalization.CultureInfo.InvariantCulture) });
        await using var clientHub = await client.ConnectUiHubAsync(ct);
        var linked = Link.Waiter(clientHub, "ClientDevicesChanged", d =>
            d.GetProperty("link").GetString() == "connected" && d.GetProperty("devices").GetArrayLength() == 1);
        var attached = Link.Waiter(clientHub, "ClientDevicesChanged", d =>
            d.GetProperty("devices").EnumerateArray().Any(x => x.GetProperty("port").ValueKind == JsonValueKind.Number));

        await Link.ConnectAsync(_server, client, ct);
        var devices = await Link.WithLastAsync(linked, clientHub, ct);
        devices.GetProperty("gateway").GetBoolean().Should().BeFalse("Koppel drives the driver itself, so the UI needs no usbip command");

        using var ui = client.CreateUiClient();
        (await ui.PostAsync("/api/client/devices/1-1.4/attach", null, ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);
        var pushed = await Link.WithLastAsync(attached, clientHub, ct);
        var entry = pushed.GetProperty("devices")[0];
        entry.GetProperty("attachedHere").GetBoolean().Should().BeTrue();
        entry.GetProperty("port").GetInt32().Should().Be(FakeUsbIpDriver.DriverPort);
        entry.GetProperty("attachedVia").ValueKind.Should().Be(JsonValueKind.Null);
        await EchoesAsync(await driver.Connection.Task.WaitAsync(TimeSpan.FromSeconds(5), ct), ct);

        var detached = Link.Waiter(clientHub, "ClientDevicesChanged", d =>
            !d.GetProperty("devices")[0].GetProperty("attachedHere").GetBoolean());
        (await ui.PostAsync("/api/client/devices/1-1.4/detach", null, ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);
        await Link.WithLastAsync(detached, clientHub, ct);
        driver.Detached.Should().Equal("1-1.4@1");
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

    /// <summary>Links the client to the server and attaches the dongle from the client's UI, as a person would.</summary>
    private async Task AttachAsync(CancellationToken ct)
    {
        await using var clientHub = await _client.ConnectUiHubAsync(ct);
        var linked = Link.Waiter(clientHub, "ClientDevicesChanged", d =>
            d.GetProperty("link").GetString() == "connected" && d.GetProperty("devices").GetArrayLength() == 1);
        var attached = Link.Waiter(clientHub, "ClientDevicesChanged", d =>
            d.GetProperty("devices").EnumerateArray().Any(x => x.GetProperty("attachedHere").GetBoolean()));
        await Link.ConnectAsync(_server, _client, ct);
        await Link.WithLastAsync(linked, clientHub, ct);

        using var ui = _client.CreateUiClient();
        using var attach = await ui.PostAsync("/api/client/devices/1-1.4/attach", null, ct);
        attach.StatusCode.Should().Be(HttpStatusCode.Accepted);
        await attached.WaitAsync(TimeSpan.FromSeconds(20), ct);
    }

    /// <summary>Sends a fake URB as the USB driver would and expects the fake server kernel to echo it.</summary>
    internal static async Task EchoesAsync(Stream driver, CancellationToken ct)
    {
        var urb = Encoding.ASCII.GetBytes("CMD_SUBMIT fake urb");
        await driver.WriteAsync(urb, ct);
        var echoed = new byte[urb.Length];
        await driver.ReadExactlyAsync(echoed, ct).AsTask().WaitAsync(TimeSpan.FromSeconds(10), ct);
        echoed.Should().Equal(urb);
    }
}
