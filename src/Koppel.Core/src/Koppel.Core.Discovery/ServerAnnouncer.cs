using System.Net;
using System.Net.Sockets;
using Microsoft.Extensions.Logging;

namespace Koppel.Core.Discovery;

/// <summary>
/// Answers discovery probes for this server on UDP, on the same port number as its HTTPS. See
/// <see cref="KoppelService"/>.
/// </summary>
public sealed partial class ServerAnnouncer(ILogger<ServerAnnouncer> logger) : IDisposable
{
    private readonly Lock _lock = new();
    private Socket? _socket;
    private CancellationTokenSource? _stop;

    public bool IsAnnouncing
    {
        get
        {
            lock (_lock)
            {
                return _socket is not null;
            }
        }
    }

    /// <summary>Starts answering, or replaces what the answers say.</summary>
    public void Announce(ServerAnnouncement announcement)
    {
        ArgumentNullException.ThrowIfNull(announcement);
        lock (_lock)
        {
            StopLocked();
            var socket = new Socket(AddressFamily.InterNetwork, SocketType.Dgram, ProtocolType.Udp);
            try
            {
                socket.Bind(new IPEndPoint(IPAddress.Any, announcement.HttpsPort));
            }
            catch
            {
                socket.Dispose();
                throw;
            }

            _socket = socket;
            _stop = new CancellationTokenSource();
            _ = AnswerAsync(socket, KoppelService.CreateReply(announcement), _stop.Token);
            LogAnnouncing(announcement.Name, announcement.HttpsPort);
        }
    }

    public void Stop()
    {
        lock (_lock)
        {
            StopLocked();
        }
    }

    public void Dispose() => Stop();

    private void StopLocked()
    {
        if (_socket is null)
        {
            return;
        }

        _stop?.Cancel();
        _stop?.Dispose();
        _socket.Dispose();
        _stop = null;
        _socket = null;
        LogStopped();
    }

    private async Task AnswerAsync(Socket socket, byte[] reply, CancellationToken cancellationToken)
    {
        var buffer = new byte[2048];
        var anyone = new IPEndPoint(IPAddress.Any, 0);
        while (!cancellationToken.IsCancellationRequested)
        {
            try
            {
                var received = await socket.ReceiveFromAsync(buffer, SocketFlags.None, anyone, cancellationToken);
                if (KoppelService.IsProbe(buffer.AsSpan(0, received.ReceivedBytes)) && reply.Length <= received.ReceivedBytes)
                {
                    await socket.SendToAsync(reply, SocketFlags.None, received.RemoteEndPoint, cancellationToken);
                }
            }
            catch (Exception ex) when (ex is OperationCanceledException or ObjectDisposedException)
            {
                return;
            }
            catch (SocketException ex) when (ex.SocketErrorCode == SocketError.ConnectionReset)
            {
                // Windows reports an earlier answer that could not be delivered on the next receive.
            }
            catch (SocketException ex)
            {
                LogAnswerFailed(ex);
                return;
            }
        }
    }

    [LoggerMessage(Level = LogLevel.Information, Message = "Answering discovery on the local network as {Name} (UDP port {Port})")]
    private partial void LogAnnouncing(string name, int port);

    [LoggerMessage(Level = LogLevel.Information, Message = "Stopped answering discovery on the local network")]
    private partial void LogStopped();

    [LoggerMessage(Level = LogLevel.Warning, Message = "Stopped answering discovery after a network error")]
    private partial void LogAnswerFailed(Exception exception);
}
