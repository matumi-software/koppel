using System.Buffers.Binary;
using System.Diagnostics;
using System.Net;
using System.Net.Sockets;

namespace Koppel.Core.Usb.Tests;

/// <summary>The upstream host against a fake USB/IP server: cached lists, one deadline, capped input, relayed export.</summary>
public sealed class UpstreamHostTests : IDisposable
{
    private static readonly TimeSpan Deadline = TimeSpan.FromMilliseconds(300);
    private readonly TcpListener _listener = new(IPAddress.Loopback, 0);
    private readonly CancellationTokenSource _stop = new();
    private readonly ManualTime _time = new();
    private Func<TcpClient, Task> _serve = _ => Task.CompletedTask;

    public UpstreamHostTests()
    {
        _listener.Start();
        _ = Task.Run(async () =>
        {
            while (!_stop.IsCancellationRequested)
            {
                try
                {
                    var tcp = await _listener.AcceptTcpClientAsync(_stop.Token);
                    _ = Task.Run(async () =>
                    {
                        using (tcp)
                        {
                            try
                            {
                                await _serve(tcp);
                            }
                            catch (Exception ex) when (ex is IOException or OperationCanceledException or ObjectDisposedException)
                            {
                                // The host hung up.
                            }
                        }
                    });
                }
                catch (Exception ex) when (ex is OperationCanceledException or SocketException or ObjectDisposedException)
                {
                    return;
                }
            }
        });
    }

    private int Port => ((IPEndPoint)_listener.LocalEndpoint).Port;

    public void Dispose()
    {
        _stop.Cancel();
        _listener.Stop();
        _stop.Dispose();
    }

    [Fact]
    public async Task Lists_what_the_server_exports_and_leaves_hubs_out()
    {
        var ct = TestContext.Current.CancellationToken;
        _serve = tcp => AnswerListAsync(tcp, [Device("2-3"), Device("2-4") with { Class = 0x09 }]);
        var host = new UsbIpUpstreamHost("127.0.0.1", Port, _time, Deadline);

        var devices = await host.ListDevicesAsync(ct);

        devices.Select(d => d.BusId).Should().Equal("2-3");
        host.Unavailable.Should().BeNull();
        host.CanShare.Should().BeFalse();
        host.CanUnshare.Should().BeFalse();
    }

    [Fact]
    public async Task A_server_that_never_answers_ends_the_refresh_at_its_deadline()
    {
        var ct = TestContext.Current.CancellationToken;
        _serve = _ => Task.Delay(Timeout.Infinite, _stop.Token);
        var host = new UsbIpUpstreamHost("127.0.0.1", Port, _time, Deadline);

        var started = Stopwatch.StartNew();
        var devices = await host.ListDevicesAsync(ct);

        devices.Should().BeEmpty();
        host.Unavailable.Should().Contain("did not answer in time");
        started.Elapsed.Should().BeLessThan(TimeSpan.FromSeconds(5));
    }

    [Fact]
    public async Task Getters_return_the_cached_list_while_a_refresh_hangs()
    {
        var ct = TestContext.Current.CancellationToken;
        var answers = 0;
        _serve = async tcp =>
        {
            if (Interlocked.Increment(ref answers) == 1)
            {
                await AnswerListAsync(tcp, [Device("2-3")]);
                return;
            }

            await Task.Delay(Timeout.Infinite, _stop.Token);
        };
        var host = new UsbIpUpstreamHost("127.0.0.1", Port, _time, TimeSpan.FromSeconds(30));
        (await host.ListDevicesAsync(ct)).Should().ContainSingle();

        // The list is stale now, so the next call starts a refresh that never finishes. Nobody waits for it.
        _time.Advance(TimeSpan.FromMinutes(1));
        var started = Stopwatch.StartNew();
        var calls = await Task.WhenAll(Enumerable.Range(0, 20).Select(_ => host.ListDevicesAsync(ct)));

        started.Elapsed.Should().BeLessThan(TimeSpan.FromSeconds(2));
        calls.Should().OnlyContain(list => list.Count == 1);
        host.Unavailable.Should().BeNull();
    }

    [Fact]
    public async Task A_list_with_more_devices_than_the_cap_is_refused()
    {
        var ct = TestContext.Current.CancellationToken;
        _serve = async tcp =>
        {
            var stream = tcp.GetStream();
            await UsbIpProtocol.ReadHeaderAsync(stream, _stop.Token);
            var message = new byte[UsbIpProtocol.HeaderSize + 4];
            UsbIpProtocol.Header(UsbIpProtocol.OpRepDevList, UsbIpProtocol.StatusOk).CopyTo(message, 0);
            BinaryPrimitives.WriteUInt32BigEndian(message.AsSpan(UsbIpProtocol.HeaderSize), 1_000_000);
            await stream.WriteAsync(message, _stop.Token);
            await Task.Delay(Timeout.Infinite, _stop.Token);
        };
        var host = new UsbIpUpstreamHost("127.0.0.1", Port, _time, Deadline);

        var devices = await host.ListDevicesAsync(ct);

        devices.Should().BeEmpty();
        host.Unavailable.Should().Contain("limit");
    }

    [Fact]
    public async Task Bus_ids_that_are_not_bus_ids_are_dropped()
    {
        var ct = TestContext.Current.CancellationToken;
        _serve = tcp => AnswerListAsync(tcp, [Device("2-3"), Device("../../etc")]);
        var host = new UsbIpUpstreamHost("127.0.0.1", Port, _time, Deadline);

        (await host.ListDevicesAsync(ct)).Select(d => d.BusId).Should().Equal("2-3");
    }

    [Fact]
    public async Task Export_relays_the_network_stream_straight_to_the_server_and_reports_it_in_use()
    {
        var ct = TestContext.Current.CancellationToken;
        var device = Device("2-3");
        _serve = async tcp =>
        {
            var stream = tcp.GetStream();
            var request = await UsbIpProtocol.ReadHeaderAsync(stream, _stop.Token);
            if (request.Code == UsbIpProtocol.OpReqDevList)
            {
                await stream.WriteAsync(UsbIpProtocol.DevListReply([device]), _stop.Token);
                return;
            }

            await UsbIpProtocol.ReadBusIdAsync(stream, _stop.Token);
            await stream.WriteAsync(UsbIpProtocol.ImportReply(device), _stop.Token);
            var buffer = new byte[64];
            int read;
            while ((read = await stream.ReadAsync(buffer, _stop.Token)) > 0)
            {
                await stream.WriteAsync(buffer.AsMemory(0, read), _stop.Token);
            }
        };
        var host = new UsbIpUpstreamHost("127.0.0.1", Port, _time, Deadline);
        await host.ListDevicesAsync(ct);
        var (client, network) = await UsbSockets.CreateLoopbackPairAsync(ct);
        using var clientSocket = client;
        await using var clientStream = new NetworkStream(clientSocket, ownsSocket: false);
        await using var networkStream = new NetworkStream(network, ownsSocket: true);
        var exported = new TaskCompletionSource();

        var session = host.ExportAsync(device, networkStream, _ =>
        {
            exported.SetResult();
            return Task.CompletedTask;
        }, ct);
        await exported.Task.WaitAsync(TimeSpan.FromSeconds(5), ct);
        await clientStream.WriteAsync("ping"u8.ToArray(), ct);
        var echoed = new byte[4];
        await clientStream.ReadExactlyAsync(echoed, ct);

        System.Text.Encoding.ASCII.GetString(echoed).Should().Be("ping");
        (await host.GetStatusAsync("2-3", ct)).Should().Be(ExportStatus.Used);
        clientSocket.Dispose();
        await session.WaitAsync(TimeSpan.FromSeconds(5), ct);
        (await host.GetStatusAsync("2-3", ct)).Should().Be(ExportStatus.Available);
    }

    private static UsbDevice Device(string busId) => new(
        BusId: busId, Path: "USB\\VID_064F&PID_2AF9\\0", BusNum: 2, DevNum: 3, Speed: UsbSpeed.Full,
        VendorId: 0x064f, ProductId: 0x2af9, BcdDevice: 0x0100, Class: 0, SubClass: 0, Protocol: 0,
        ConfigurationValue: 1, NumConfigurations: 1, Interfaces: [new UsbInterface(0xff, 0, 0)],
        Manufacturer: null, Product: null, Serial: null, Driver: null);

    private async Task AnswerListAsync(TcpClient tcp, IReadOnlyCollection<UsbDevice> devices)
    {
        var stream = tcp.GetStream();
        await UsbIpProtocol.ReadHeaderAsync(stream, _stop.Token);
        await stream.WriteAsync(UsbIpProtocol.DevListReply(devices), _stop.Token);
    }

    /// <summary>A clock the test moves by hand. Timers still run on the system clock.</summary>
    private sealed class ManualTime : TimeProvider
    {
        private DateTimeOffset _now = new(2026, 1, 1, 0, 0, 0, TimeSpan.Zero);

        public override DateTimeOffset GetUtcNow() => _now;

        public void Advance(TimeSpan by) => _now += by;
    }
}
