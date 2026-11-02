using System.Collections.Concurrent;
using System.Net.WebSockets;
using System.Security.Claims;
using Koppel.Core.Activity;
using Koppel.Core.Auth;
using Koppel.Core.Usb;
using Koppel.Server;
using Koppel.Settings;

namespace Koppel.Usb;

/// <summary>
/// The encrypted USB data plane (server role). A client opens a WebSocket at <c>/api/remote/usb</c> on
/// the HTTPS port, pinned to the server's certificate and carrying its bearer token, then speaks plain
/// USB/IP inside it. On import the host relays the stream to the device, so USB traffic never crosses
/// the network unencrypted, and Koppel needs no port of its own for it.
/// </summary>
/// <remarks>
/// Kestrel and the remote group do what the USB port's own listener used to: TLS, the token, the
/// client's generation (<see cref="ServerSecretsStore.IsCurrent(ClaimsPrincipal?)"/>), and the surface
/// guard, which only lets the request in while the role is server or both. A session ends when its
/// client is revoked or its secret is rotated (see <see cref="LiveSessions"/>), when the role leaves the
/// server side, when either end stops answering pings, or when Koppel stops.
/// </remarks>
internal sealed partial class UsbDataPlane(
    UsbServer usb,
    ServerSecretsStore secrets,
    LiveSessions live,
    IActivityRecorder activity,
    SettingsStore settings,
    ILogger<UsbDataPlane> logger) : IHostedService, IDisposable
{
    /// <summary>
    /// Both ends ping this often, and a peer that has not answered within <see cref="KeepAliveTimeout"/> is
    /// gone: a sleeping laptop or a pulled cable frees the device in under a minute, not at the next restart.
    /// </summary>
    public static readonly TimeSpan KeepAliveInterval = TimeSpan.FromSeconds(15);

    public static readonly TimeSpan KeepAliveTimeout = TimeSpan.FromSeconds(30);

    /// <summary>Time for an authenticated client to ask for the device list or an import.</summary>
    private static readonly TimeSpan RequestBudget = TimeSpan.FromSeconds(10);

    private static readonly TimeSpan StopGrace = TimeSpan.FromSeconds(5);

    private readonly ConcurrentDictionary<Task, byte> _sessions = new();
    private readonly ConcurrentDictionary<string, SemaphoreSlim> _busLocks = new(StringComparer.Ordinal);
    private CancellationTokenSource _serving = new();

    public Task StartAsync(CancellationToken cancellationToken)
    {
        settings.Changed += OnSettingsAsync;
        return Task.CompletedTask;
    }

    public async Task StopAsync(CancellationToken cancellationToken)
    {
        settings.Changed -= OnSettingsAsync;
        await EndServingAsync();
    }

    public void Dispose() => _serving.Dispose();

    /// <summary>Serves one WebSocket for a client the remote group already let in, until the session ends.</summary>
    public async Task ServeAsync(Stream session, ClaimsPrincipal user, CancellationToken requestAborted)
    {
        using var abort = CancellationTokenSource.CreateLinkedTokenSource(requestAborted);
        CancellationTokenRegistration serving;
        try
        {
            serving = Volatile.Read(ref _serving).Token.Register(abort.Cancel);
        }
        catch (ObjectDisposedException)
        {
            // The role changed as this session started; it ends like the others did.
            return;
        }

        using (serving)
        using (live.Add(user.GetClientId()!, user.GetGeneration()!.Value, abort.Cancel))
        {
            var run = ServeCoreAsync(session, user.GetClientId()!, abort.Token);
            Track(run);
            await run;
        }
    }

    private async Task ServeCoreAsync(Stream session, string clientId, CancellationToken cancellationToken)
    {
        try
        {
            UsbIpProtocol.OpHeader header;
            string? busId = null;
            using (var request = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken))
            {
                request.CancelAfter(RequestBudget);
                header = await UsbIpProtocol.ReadHeaderAsync(session, request.Token);
                if (header.Code == UsbIpProtocol.OpReqImport)
                {
                    busId = await UsbIpProtocol.ReadBusIdAsync(session, request.Token);
                }
            }

            if (header.Code == UsbIpProtocol.OpReqDevList)
            {
                await session.WriteAsync(UsbIpProtocol.DevListReply(await usb.ExportableAsync(cancellationToken)), cancellationToken);
            }
            else if (busId is not null)
            {
                await ImportAsync(session, busId, clientId, cancellationToken);
            }
        }
        catch (OperationCanceledException)
        {
            LogSessionEnded("it was cancelled or ran out of time");
        }
        catch (Exception ex) when (ex is IOException or WebSocketException or EndOfStreamException or ObjectDisposedException)
        {
            LogSessionEnded(ex.Message);
        }
        catch (Exception ex)
        {
            // One bad session must not take the host down, and it must leave a trace.
            LogSessionFailed(ex);
        }
    }

    private async Task ImportAsync(Stream session, string busId, string clientId, CancellationToken cancellationToken)
    {
        if (!UsbBusId.IsValid(busId))
        {
            await session.WriteAsync(UsbIpProtocol.ImportReply(null), cancellationToken);
            return;
        }

        // The status check, the reset and the start of the export are one step per device. Without it, two
        // imports can both see the device available, and the second one's reset hits the first one's session.
        // The lock is let go as soon as the export has begun, when the device reads as in use.
        var busLock = _busLocks.GetOrAdd(busId, _ => new SemaphoreSlim(1, 1));
        await busLock.WaitAsync(cancellationToken);
        var locked = 1;
        void Unlock()
        {
            if (Interlocked.Exchange(ref locked, 0) == 1)
            {
                busLock.Release();
            }
        }

        try
        {
            var host = usb.Host;
            var device = (await usb.ExportableAsync(cancellationToken)).FirstOrDefault(d => d.BusId == busId);
            if (device is null || await host.GetStatusAsync(busId, cancellationToken) != ExportStatus.Available)
            {
                await session.WriteAsync(UsbIpProtocol.ImportReply(null), cancellationToken);
                return;
            }

            if (host.CanReset)
            {
                try
                {
                    await host.ResetAsync(busId, cancellationToken);
                }
                catch (UsbHostException ex)
                {
                    // Most devices work without it, so the attach goes ahead.
                    LogResetFailed(busId, ex.Message);
                }
            }

            var clientName = secrets.NameOf(clientId) ?? clientId;
            var deviceName = device.Product ?? busId;
            var attached = false;
            try
            {
                // Returns when the client detaches or the link drops; the host then frees the device.
                await host.ExportAsync(device, session, async token =>
                {
                    Unlock();
                    await session.WriteAsync(UsbIpProtocol.ImportReply(device), token);
                    attached = true;
                    LogAttached(busId, clientId);
                    await activity.RecordAsync(new EntityRef(busId, deviceName, "device"), ActivityReason.Attached, ActivityOutcome.Succeeded, $"{clientName} attached {deviceName}", clientId);
                    await usb.SetAttachedAsync(busId, clientId);
                }, cancellationToken);
            }
            catch (UsbHostException ex) when (!attached)
            {
                LogExportFailed(busId, ex.Message);
                await session.WriteAsync(UsbIpProtocol.ImportReply(null), CancellationToken.None);
            }
            finally
            {
                if (attached)
                {
                    LogDetached(busId, clientId);
                    await activity.RecordAsync(new EntityRef(busId, deviceName, "device"), ActivityReason.Detached, ActivityOutcome.Succeeded, $"{clientName} released {deviceName}", clientId);
                    await usb.SetAttachedAsync(busId, null);
                }
            }
        }
        finally
        {
            Unlock();
        }
    }

    private static bool Serves(KoppelRole role) => role is KoppelRole.Server or KoppelRole.Both;

    /// <summary>A machine that stops being a server ends what it serves. New sessions are kept out by the surface guard.</summary>
    private Task OnSettingsAsync(KoppelSettings next) => Serves(next.Role) ? Task.CompletedTask : EndServingAsync();

    private void Track(Task session)
    {
        _sessions[session] = 0;
        _ = session.ContinueWith(done => _sessions.TryRemove(done, out _), TaskScheduler.Default);
    }

    /// <summary>Ends every session and waits a little for them to finish, so nothing outlives its owner.</summary>
    private async Task EndServingAsync()
    {
        var ending = Interlocked.Exchange(ref _serving, new CancellationTokenSource());
        await ending.CancelAsync();
        var running = _sessions.Keys.ToArray();
        try
        {
            if (running.Length > 0)
            {
                await Task.WhenAll(running).WaitAsync(StopGrace);
            }
        }
        catch (TimeoutException)
        {
            LogSessionsStillRunning(_sessions.Count);
        }
        finally
        {
            ending.Dispose();
        }
    }

    [LoggerMessage(Level = LogLevel.Information, Message = "Device {BusId} attached by {ClientId}")]
    private partial void LogAttached(string busId, string clientId);

    [LoggerMessage(Level = LogLevel.Information, Message = "Device {BusId} released by {ClientId}")]
    private partial void LogDetached(string busId, string clientId);

    [LoggerMessage(Level = LogLevel.Warning, Message = "Could not reset {BusId} before export: {Reason}")]
    private partial void LogResetFailed(string busId, string reason);

    [LoggerMessage(Level = LogLevel.Warning, Message = "Export of {BusId} failed: {Reason}")]
    private partial void LogExportFailed(string busId, string reason);

    [LoggerMessage(Level = LogLevel.Debug, Message = "USB session ended: {Reason}")]
    private partial void LogSessionEnded(string reason);

    [LoggerMessage(Level = LogLevel.Warning, Message = "A USB session failed")]
    private partial void LogSessionFailed(Exception exception);

    [LoggerMessage(Level = LogLevel.Warning, Message = "{Count} USB sessions were still running when serving stopped")]
    private partial void LogSessionsStillRunning(int count);
}
