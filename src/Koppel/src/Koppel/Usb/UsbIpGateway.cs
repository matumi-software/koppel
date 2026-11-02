using System.Net;
using System.Net.Sockets;

namespace Koppel.Usb;

/// <summary>
/// Client role, off by default: a plain USB/IP server for a USB/IP driver on this machine, such as
/// usbip-win2 on the Windows host of Koppel's container. A container cannot hand a device to the
/// Windows kernel, but the host's driver can attach from here. Each connection is carried to the
/// server through <see cref="UsbClient"/>'s TLS session, so the network only sees encrypted traffic.
/// </summary>
/// <remarks>
/// Anyone who reaches this port gets this client's access to the server, with no login. It listens on
/// <c>Gateway:Address</c> (127.0.0.1 unless set) and <c>Gateway:Port</c>. In a container, listen on all
/// addresses and publish the port on the host's 127.0.0.1 only.
/// </remarks>
internal sealed partial class UsbIpGateway(UsbClient client, IConfiguration configuration, ILogger<UsbIpGateway> logger) : BackgroundService
{
    protected override async Task ExecuteAsync(CancellationToken stoppingToken)
    {
        var port = configuration.GetValue("Gateway:Port", 0);
        if (port == 0)
        {
            return;
        }

        var address = IPAddress.Parse(configuration["Gateway:Address"] ?? "127.0.0.1");
        using var listener = new Socket(address.AddressFamily, SocketType.Stream, ProtocolType.Tcp);
        listener.Bind(new IPEndPoint(address, port));
        listener.Listen(16);
        LogListening(address, port);
        await client.SetGatewayAsync(true);
        try
        {
            while (true)
            {
                var socket = await listener.AcceptAsync(stoppingToken);
                socket.NoDelay = true;
                _ = Task.Run(() => ServeAsync(socket, stoppingToken), CancellationToken.None);
            }
        }
        finally
        {
            await client.SetGatewayAsync(false);
        }
    }

    private async Task ServeAsync(Socket socket, CancellationToken stoppingToken)
    {
        try
        {
            await client.ServeGatewayAsync(socket, stoppingToken);
        }
        catch (Exception ex) when (ex is IOException or SocketException or OperationCanceledException or ObjectDisposedException)
        {
            LogSessionEnded(ex.Message);
        }
    }

    [LoggerMessage(Level = LogLevel.Information, Message = "USB/IP gateway listening on {Address}:{Port}")]
    private partial void LogListening(IPAddress address, int port);

    [LoggerMessage(Level = LogLevel.Debug, Message = "Gateway session ended: {Reason}")]
    private partial void LogSessionEnded(string reason);
}
