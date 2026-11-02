using System.Net;
using System.Net.Sockets;
using Koppel.Core.Usb;
using Microsoft.Extensions.DependencyInjection;

namespace Koppel.Tests.Usb;

/// <summary>
/// A Koppel server that shares a plain USB/IP server's devices (as with usbipd-win on a Windows host),
/// and a Koppel client that attaches one. The USB/IP server is played by <see cref="FakeUsbIpServer"/>.
/// </summary>
public sealed class UpstreamTests : IDisposable
{
    private readonly FakeUsbIpServer _usbipd = new();
    private readonly FakeKernel _clientKernel = new();
    private readonly KoppelFactory _server;
    private readonly KoppelFactory _client;

    public UpstreamTests()
    {
        _server = new KoppelFactory(kestrel: true, configuration: new Dictionary<string, string> { ["Usb:Upstream"] = $"127.0.0.1:{_usbipd.Port}" });
        _client = new KoppelFactory(kestrel: true, services: s => s.AddSingleton<IUsbAttacher>(_clientKernel));
    }

    public void Dispose()
    {
        _client.Dispose();
        _server.Dispose();
        _usbipd.Dispose();
    }

    [Fact]
    public async Task Client_attaches_a_device_the_usbip_server_exports_and_traffic_reaches_it()
    {
        var ct = TestContext.Current.CancellationToken;
        await using var clientHub = await _client.ConnectUiHubAsync(ct);
        var linked = Link.Waiter(clientHub, "ClientDevicesChanged", d =>
            d.GetProperty("link").GetString() == "connected"
            && d.GetProperty("devices").EnumerateArray().Any(x => x.GetProperty("device").GetProperty("state").GetString() == "shared"));
        var attached = Link.Waiter(clientHub, "ClientDevicesChanged", d =>
            d.GetProperty("devices").EnumerateArray().Any(x => x.GetProperty("attachedHere").GetBoolean()));

        await Link.ConnectAsync(_server, _client, ct);
        var devices = await Link.WithLastAsync(linked, clientHub, ct);
        devices.GetProperty("devices")[0].GetProperty("device").GetProperty("busId").GetString().Should().Be(FakeUsbIpServer.BusId);

        using var ui = _client.CreateUiClient();
        using var attach = await ui.PostAsync($"/api/client/devices/{FakeUsbIpServer.BusId}/attach", null, ct);
        attach.StatusCode.Should().Be(HttpStatusCode.Accepted);
        await Link.WithLastAsync(attached, clientHub, ct);

        var socket = await _clientKernel.AttachedSocket.Task.WaitAsync(TimeSpan.FromSeconds(5), ct);
        await using var driver = new NetworkStream(socket, ownsSocket: false);
        await UsbEndToEndTests.EchoesAsync(driver, ct);
        _usbipd.Imports.Should().Equal(FakeUsbIpServer.BusId);
    }

    /// <summary>Plays usbipd-win: lists one exported device and echoes USB traffic after an import.</summary>
    private sealed class FakeUsbIpServer : IDisposable
    {
        public const string BusId = "2-3";

        private static readonly UsbDevice Device = FakeKernel.Dongle with { BusId = BusId, Path = @"USB\VID_064F&PID_2AF9\0", Driver = null, Product = null, Manufacturer = null };

        private readonly TcpListener _listener = new(IPAddress.Loopback, 0);
        private readonly CancellationTokenSource _stop = new();

        public FakeUsbIpServer()
        {
            _listener.Start();
            _ = Task.Run(AcceptAsync);
        }

        public int Port => ((IPEndPoint)_listener.LocalEndpoint).Port;

        public List<string> Imports { get; } = [];

        public void Dispose()
        {
            _stop.Cancel();
            _listener.Stop();
            _stop.Dispose();
        }

        private async Task AcceptAsync()
        {
            while (!_stop.IsCancellationRequested)
            {
                TcpClient tcp;
                try
                {
                    tcp = await _listener.AcceptTcpClientAsync(_stop.Token);
                }
                catch (Exception ex) when (ex is OperationCanceledException or SocketException or ObjectDisposedException)
                {
                    return;
                }

                _ = Task.Run(() => ServeAsync(tcp));
            }
        }

        private async Task ServeAsync(TcpClient tcp)
        {
            using (tcp)
            {
                var stream = tcp.GetStream();
                try
                {
                    var request = await UsbIpProtocol.ReadHeaderAsync(stream, _stop.Token);
                    if (request.Code == UsbIpProtocol.OpReqDevList)
                    {
                        await stream.WriteAsync(UsbIpProtocol.DevListReply([Device]), _stop.Token);
                        return;
                    }

                    var busId = await UsbIpProtocol.ReadBusIdAsync(stream, _stop.Token);
                    lock (Imports)
                    {
                        Imports.Add(busId);
                    }

                    await stream.WriteAsync(UsbIpProtocol.ImportReply(busId == BusId ? Device : null), _stop.Token);
                    var buffer = new byte[4096];
                    int read;
                    while ((read = await stream.ReadAsync(buffer, _stop.Token)) > 0)
                    {
                        await stream.WriteAsync(buffer.AsMemory(0, read), _stop.Token);
                    }
                }
                catch (Exception ex) when (ex is IOException or OperationCanceledException or ObjectDisposedException)
                {
                    // Session over.
                }
            }
        }
    }
}
