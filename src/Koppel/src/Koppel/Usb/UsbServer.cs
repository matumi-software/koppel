using System.Collections.Concurrent;
using System.Text.Json;
using System.Threading.Channels;
using Koppel.Core.Activity;
using Koppel.Core.Configuration;
using Koppel.Core.Usb;
using Koppel.Settings;

namespace Koppel.Usb;

/// <summary>The bus IDs the user chose to share, kept in the plain <c>devices.json</c>.</summary>
/// <param name="VidPids">The VID:PID each bus ID had when it was shared. A bus ID is a port, so a different
/// device plugged into the same port is not shared without asking.</param>
internal sealed record SharedDevices(IReadOnlyList<string> BusIds, IReadOnlyDictionary<string, string>? VidPids = null)
{
    public static SharedDevices Empty { get; } = new([]);

    public bool Expects(UsbDevice device) =>
        BusIds.Contains(device.BusId) && (VidPids?.GetValueOrDefault(device.BusId) is not { } vidPid || vidPid == device.VidPid);
}

/// <summary>A device as the UI sees it. <see cref="State"/> uses the design system's state words.</summary>
/// <param name="State"><c>local</c>, <c>shared</c>, <c>attached</c> or <c>offline</c> (shared but unplugged).</param>
/// <param name="Fault">Why the last change to this device failed, for the UI.</param>
public sealed record DeviceView(
    string BusId,
    string VidPid,
    string Name,
    string? Manufacturer,
    string Speed,
    string State,
    string? AttachedTo,
    DateTimeOffset? AttachedSince,
    string? Fault,
    string? Serial = null,
    string? Driver = null,
    IReadOnlyList<string>? Interfaces = null);

/// <summary>What the server's UI shows: its devices, or why it cannot share any.</summary>
public sealed record ServerDevices(IReadOnlyList<DeviceView> Devices, string? Unavailable);

/// <summary>
/// Server role: tracks the USB devices on this machine, applies the user's share choices to the
/// kernel, and raises <see cref="Changed"/> whenever what the kernel reports changes. Polls the host
/// every two seconds, which also catches plugging and unplugging.
/// </summary>
/// <remarks>
/// A snapshot is taken, and given a sequence number, under a lock. It is delivered to the handlers outside
/// that lock, by one pump, in order, and a snapshot that a newer one overtook while the handlers were busy is
/// dropped. Handlers push to the network, so a slow or stuck peer holds up only the pump, never an import,
/// a share or the poll.
/// </remarks>
internal sealed partial class UsbServer(
    IUsbHost host,
    KoppelPaths paths,
    SettingsStore settings,
    TimeProvider time,
    IActivityRecorder activity,
    ILogger<UsbServer> logger) : BackgroundService
{
    private static readonly TimeSpan PollInterval = TimeSpan.FromSeconds(2);

    private readonly JsonConfigFile<SharedDevices> _shared = new(
        Path.Combine(paths.ConfigDirectory, "devices.json"), SharedDevices.Empty, KoppelJson.Files, logger, startWithDefaultsWhenUnusable: true);

    /// <summary>Why <c>devices.json</c> is not being used, or null. Koppel started with nothing shared.</summary>
    public string? SharedProblem => _shared.Problem;

    private readonly ConcurrentDictionary<string, string> _faults = new();
    private readonly ConcurrentDictionary<string, (string ClientId, DateTimeOffset Since)> _attachedTo = new();
    private readonly ConcurrentDictionary<string, string> _names = new();

    /// <summary>The bus IDs Koppel means to have shared right now. Saved shares come back into it at start-up only when <c>ShareOnStartup</c> is on.</summary>
    private readonly ConcurrentDictionary<string, byte> _active = new();

    private readonly SemaphoreSlim _gate = new(1, 1);
    private readonly SemaphoreSlim _applyGate = new(1, 1);
    private readonly Channel<Delivery> _deliveries = Channel.CreateUnbounded<Delivery>(new UnboundedChannelOptions { SingleReader = true });
    private readonly WakeSignal _refreshRequested = new();
    private readonly BackgroundTasks _tasks = new(logger);
    private long _sequence;
    private KoppelRole _role;
    private HashSet<string>? _present;
    private string _lastJson = "";
    private ServerDevices _current = new([], null);

    public event Func<ServerDevices, Task>? Changed;

    public ServerDevices Current => Volatile.Read(ref _current);

    /// <summary>Devices a client may import: bound for export and the ones the user shared.</summary>
    public async Task<IReadOnlyList<UsbDevice>> ExportableAsync(CancellationToken cancellationToken) =>
        [.. (await host.ListDevicesAsync(cancellationToken)).Where(IsExportable)];

    public IUsbHost Host => host;

    public override async Task StartAsync(CancellationToken cancellationToken)
    {
        await _shared.StartAsync(cancellationToken);
        _role = settings.Current.Role;
        settings.Changed += OnSettingsAsync;
        try
        {
            await ReconcileAsync(cancellationToken);
            await RefreshAsync(cancellationToken);
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        {
            // A sysfs hiccup at start-up is no reason to stay down. The poll applies the saved shares and looks again.
            LogStartUpFailed(ex);
        }

        await base.StartAsync(cancellationToken);
    }

    public override async Task StopAsync(CancellationToken cancellationToken)
    {
        settings.Changed -= OnSettingsAsync;
        _shared.Stop();
        await base.StopAsync(cancellationToken);
        await _tasks.WaitAsync(TimeSpan.FromSeconds(5));
    }

    /// <summary>
    /// Applies the choice to the kernel, and only when that worked records it in <c>devices.json</c>, then
    /// raises <see cref="Changed"/> with what the kernel now reports. A kernel refusal shows as the device's
    /// fault and nothing is saved, so a share that never took effect is not applied by surprise at the next start.
    /// </summary>
    /// <exception cref="ArgumentException">The bus ID is not a USB bus ID. Nothing is changed.</exception>
    public async Task SetSharedAsync(string busId, bool share, CancellationToken cancellationToken)
    {
        if (!UsbBusId.IsValid(busId))
        {
            throw new ArgumentException("That is not a USB bus ID.", nameof(busId));
        }

        string? failure;
        await _applyGate.WaitAsync(cancellationToken);
        try
        {
            var device = (await host.ListDevicesAsync(cancellationToken)).FirstOrDefault(d => d.BusId == busId);
            // Only a device that is plugged in and can be shared is remembered. A hub is not listed, and a port with
            // nothing on it has nothing to remember, so neither may turn up later as an unplugged device.
            failure = share && device is null
                ? Fail(busId, "There is no shareable device on this bus ID. A hub cannot be shared.")
                : await ApplyChoiceAsync(busId, share, device, cancellationToken);
            if (failure is null)
            {
                await SaveChoiceAsync(busId, share, device, cancellationToken);
            }
        }
        finally
        {
            _applyGate.Release();
        }

        await RefreshAsync(cancellationToken, force: true);

        var name = _names.GetValueOrDefault(busId, busId);
        var entity = new EntityRef(busId, name, "device");
        if (failure is not null)
        {
            await activity.RecordAsync(entity, share ? ActivityReason.Shared : ActivityReason.Unshared, ActivityOutcome.Failed, $"{name} was not {(share ? "shared" : "unshared")}", failure);
        }
        else
        {
            await activity.RecordAsync(entity, share ? ActivityReason.Shared : ActivityReason.Unshared, ActivityOutcome.Succeeded, share ? $"{name} is shared" : $"{name} is no longer shared");
        }
    }

    /// <summary>Called by the data plane when a client imports or releases a device. The UI hears about it a moment later, and nobody waits for that.</summary>
    public Task SetAttachedAsync(string busId, string? clientId)
    {
        if (clientId is null)
        {
            _attachedTo.TryRemove(busId, out _);
        }
        else
        {
            _attachedTo[busId] = (clientId, time.GetUtcNow());
        }

        _refreshRequested.Set();
        return Task.CompletedTask;
    }

    public override void Dispose()
    {
        _shared.Dispose();
        _gate.Dispose();
        _applyGate.Dispose();
        base.Dispose();
    }

    protected override async Task ExecuteAsync(CancellationToken stoppingToken)
    {
        try
        {
            await Task.WhenAll(PollAsync(stoppingToken), DeliverAsync(stoppingToken), RefreshWhenRequestedAsync(stoppingToken));
        }
        catch (OperationCanceledException) when (stoppingToken.IsCancellationRequested)
        {
            // Stopping.
        }
    }

    private async Task PollAsync(CancellationToken stoppingToken)
    {
        using var timer = new PeriodicTimer(PollInterval);
        var failures = 0;
        var skip = 0;
        while (await timer.WaitForNextTickAsync(stoppingToken))
        {
            if (skip > 0)
            {
                skip--;
                continue;
            }

            try
            {
                // A shared device that comes back after unplugging needs binding again.
                // Only a server re-shares. A pure client never binds anything to usbip-host.
                if (Serves(_role))
                {
                    await _applyGate.WaitAsync(stoppingToken);
                    try
                    {
                        await ApplyIntentAsync(unshareStrays: false, stoppingToken);
                    }
                    finally
                    {
                        _applyGate.Release();
                    }
                }

                await RefreshAsync(stoppingToken);
            }
            catch (Exception ex) when (ex is not OperationCanceledException || !stoppingToken.IsCancellationRequested)
            {
                // Whatever the host throws, the poll goes on: it is what notices a plugged device and re-shares one.
                // A host that keeps failing is asked less often, up to every fifteenth tick.
                LogPollFailed(ex);
                skip = Math.Min(1 << Math.Min(failures++, 4), 14);
                continue;
            }

            failures = 0;
        }
    }

    /// <summary>Takes snapshots off the queue in order and hands the newest to the handlers. Snapshots overtaken while the handlers ran are dropped.</summary>
    private async Task DeliverAsync(CancellationToken stoppingToken)
    {
        long delivered = 0;
        while (await _deliveries.Reader.WaitToReadAsync(stoppingToken))
        {
            Delivery? latest = null;
            var plugs = new List<PlugEvent>();
            while (_deliveries.Reader.TryRead(out var next))
            {
                plugs.AddRange(next.Plugs);
                latest = next.Sequence > (latest?.Sequence ?? delivered) ? next : latest;
            }

            if (latest is null)
            {
                continue;
            }

            delivered = latest.Sequence;
            try
            {
                await Changed.RaiseAsync(latest.Devices, logger);
                foreach (var plug in plugs)
                {
                    await activity.RecordAsync(new EntityRef(plug.BusId, plug.Name, "device"), plug.Reason, ActivityOutcome.Succeeded, plug.Summary);
                }
            }
            catch (Exception ex) when (ex is not OperationCanceledException)
            {
                LogDeliveryFailed(ex);
            }
        }
    }

    /// <summary>Refreshes for the callers that only ask for it, so none of them waits on the host.</summary>
    private async Task RefreshWhenRequestedAsync(CancellationToken stoppingToken)
    {
        while (true)
        {
            await _refreshRequested.WaitAsync(stoppingToken);
            try
            {
                await RefreshAsync(stoppingToken, force: true);
            }
            catch (Exception ex) when (ex is not OperationCanceledException)
            {
                LogPollFailed(ex);
            }
        }
    }

    private static bool Serves(KoppelRole role) => role is KoppelRole.Server or KoppelRole.Both;

    /// <summary>Bound for export, and the device the user shared on that port.</summary>
    private bool IsExportable(UsbDevice device) =>
        host.IsShared(device) && (!host.CanShare || _shared.Current.Expects(device));

    /// <summary>
    /// At start-up: shares what the user saved (when <c>ShareOnStartup</c> is on and this is a server), and
    /// unbinds whatever else is bound to usbip-host, such as a share a crash or an earlier run left behind.
    /// </summary>
    private async Task ReconcileAsync(CancellationToken cancellationToken)
    {
        await _applyGate.WaitAsync(cancellationToken);
        try
        {
            ActivateSaved(settings.Current);
            await ApplyIntentAsync(unshareStrays: true, cancellationToken);
        }
        finally
        {
            _applyGate.Release();
        }
    }

    /// <summary>Makes the saved shares the ones Koppel means to have, when this machine is a server that re-shares on start.</summary>
    private void ActivateSaved(KoppelSettings current)
    {
        _active.Clear();
        if (!Serves(current.Role) || !current.ShareOnStartup)
        {
            return;
        }

        foreach (var busId in _shared.Current.BusIds)
        {
            _active[busId] = 0;
        }
    }

    /// <summary>
    /// Brings the kernel in line with what Koppel means to share. A device that is bound but is not the one the
    /// user shared on that port is unbound, and says so. With <paramref name="unshareStrays"/>, so is every
    /// other bound device Koppel does not mean to share. Call with the apply gate held.
    /// </summary>
    private async Task ApplyIntentAsync(bool unshareStrays, CancellationToken cancellationToken)
    {
        var devices = await host.ListDevicesAsync(cancellationToken);
        if (host.Unavailable is not null || !host.CanShare)
        {
            return;
        }

        var saved = _shared.Current;
        foreach (var device in devices)
        {
            if (_active.ContainsKey(device.BusId) && saved.Expects(device))
            {
                if (!host.IsShared(device) && !_faults.ContainsKey(device.BusId))
                {
                    await TryApplyAsync(device.BusId, share: true, cancellationToken);
                }
            }
            else if (host.IsShared(device) && host.CanUnshare && (unshareStrays || saved.BusIds.Contains(device.BusId)))
            {
                var wrongDevice = saved.BusIds.Contains(device.BusId) && !saved.Expects(device);
                var failure = await TryApplyAsync(device.BusId, share: false, cancellationToken);
                if (failure is null && wrongDevice)
                {
                    _faults[device.BusId] = "A different device is plugged into this port now, so it is not shared. Share it again to use it.";
                }
            }
        }
    }

    /// <summary>A settings change that moves the role in or out of the server side shares or unshares to match.</summary>
    private async Task OnSettingsAsync(KoppelSettings next)
    {
        var was = Serves(_role);
        _role = next.Role;
        if (was == Serves(next.Role))
        {
            return;
        }

        if (was)
        {
            // Giving every device back can take a while. The settings change is confirmed to the UI without it.
            _tasks.Run("unshare on leaving the server role", async () =>
            {
                await _applyGate.WaitAsync();
                try
                {
                    // The role came back while this waited: the handler for that change shares again.
                    if (!Serves(_role))
                    {
                        await UnshareEverythingAsync();
                    }
                }
                finally
                {
                    _applyGate.Release();
                }

                await RefreshAsync(CancellationToken.None, force: true);
            });
            return;
        }

        await _applyGate.WaitAsync();
        try
        {
            ActivateSaved(next);
            await ApplyIntentAsync(unshareStrays: false, CancellationToken.None);
        }
        finally
        {
            _applyGate.Release();
        }

        await RefreshAsync(CancellationToken.None, force: true);
    }

    /// <summary>A machine that is no longer a server gives back every device Koppel shared. Call with the apply gate held.</summary>
    private async Task UnshareEverythingAsync()
    {
        var saved = _shared.Current;
        var devices = await host.ListDevicesAsync(CancellationToken.None);
        _active.Clear();
        if (host.Unavailable is not null || !host.CanUnshare)
        {
            return;
        }

        foreach (var device in devices.Where(d => host.IsShared(d) && saved.BusIds.Contains(d.BusId)))
        {
            await TryApplyAsync(device.BusId, share: false, CancellationToken.None);
        }
    }

    /// <summary>What the host can do for the choice. Returns why it failed, or null when it took effect.</summary>
    private async Task<string?> ApplyChoiceAsync(string busId, bool share, UsbDevice? device, CancellationToken cancellationToken)
    {
        if (host.Unavailable is { } unavailable)
        {
            return Fail(busId, unavailable);
        }

        if (share ? host.CanShare : host.CanUnshare)
        {
            return await TryApplyAsync(busId, share, cancellationToken);
        }

        // The host shares by its own tools. A choice that already matches what it exports is nothing to apply.
        if (share != (device is not null && host.IsShared(device)))
        {
            return Fail(busId, host.SharingAdvice(busId, share));
        }

        _faults.TryRemove(busId, out _);
        return null;
    }

    /// <summary>Writes the choice to <c>devices.json</c> after it took effect. If the file cannot be written, the kernel change is undone.</summary>
    private async Task SaveChoiceAsync(string busId, bool share, UsbDevice? device, CancellationToken cancellationToken)
    {
        try
        {
            await _shared.UpdateAsync(current => current with
            {
                BusIds = share ? [.. current.BusIds.Append(busId).Distinct()] : [.. current.BusIds.Where(b => b != busId)],
                VidPids = share && device is not null
                    ? new Dictionary<string, string>(current.VidPids ?? new Dictionary<string, string>()) { [busId] = device.VidPid }
                    : current.VidPids?.Where(p => p.Key != busId).ToDictionary(p => p.Key, p => p.Value),
            }, cancellationToken);
        }
        catch
        {
            if (share ? host.CanUnshare : host.CanShare)
            {
                await TryApplyAsync(busId, !share, CancellationToken.None);
            }

            throw;
        }

        if (share)
        {
            _active[busId] = 0;
        }
        else
        {
            _active.TryRemove(busId, out _);
        }
    }

    private async Task<string?> TryApplyAsync(string busId, bool share, CancellationToken cancellationToken)
    {
        try
        {
            if (share)
            {
                await host.ShareAsync(busId, cancellationToken);
            }
            else
            {
                await host.UnshareAsync(busId, cancellationToken);
            }

            _faults.TryRemove(busId, out _);
            return null;
        }
        catch (UsbHostException ex)
        {
            return Fail(busId, ex.Message);
        }
    }

    private string Fail(string busId, string reason)
    {
        _faults[busId] = reason;
        LogApplyFailed(busId, reason);
        return reason;
    }

    private async Task RefreshAsync(CancellationToken cancellationToken, bool force = false)
    {
        // Only the snapshot and its place in the queue are taken under the gate. Handlers run in the pump.
        await _gate.WaitAsync(cancellationToken);
        try
        {
            var next = await BuildAsync(cancellationToken);
            var json = JsonSerializer.Serialize(next, KoppelJson.Files);
            if (!force && json == _lastJson)
            {
                return;
            }

            _lastJson = json;
            Volatile.Write(ref _current, next);
            _deliveries.Writer.TryWrite(new Delivery(++_sequence, next, PlugChanges(next)));
        }
        finally
        {
            _gate.Release();
        }
    }

    /// <summary>Devices that appeared or disappeared since the last look. The first look only remembers.</summary>
    private List<PlugEvent> PlugChanges(ServerDevices devices)
    {
        var present = devices.Devices.Where(d => d.State != "offline").Select(d => d.BusId).ToHashSet(StringComparer.Ordinal);
        var previous = Interlocked.Exchange(ref _present, present);
        if (previous is null)
        {
            return [];
        }

        var changes = new List<PlugEvent>();
        foreach (var busId in present.Except(previous))
        {
            var name = _names.GetValueOrDefault(busId, busId);
            changes.Add(new PlugEvent(busId, name, ActivityReason.Plugged, $"{name} was plugged in on bus {busId}"));
        }

        foreach (var busId in previous.Except(present))
        {
            var name = _names.GetValueOrDefault(busId, busId);
            changes.Add(new PlugEvent(busId, name, ActivityReason.Unplugged, $"{name} was unplugged from bus {busId}"));
        }

        return changes;
    }

    private async Task<ServerDevices> BuildAsync(CancellationToken cancellationToken)
    {
        var intent = _shared.Current.BusIds.ToHashSet(StringComparer.Ordinal);
        var devices = await host.ListDevicesAsync(cancellationToken);
        var views = new List<DeviceView>();
        foreach (var d in devices)
        {
            _names[d.BusId] = d.Product ?? "Unknown device";
            var state = IsExportable(d)
                ? await host.GetStatusAsync(d.BusId, cancellationToken) == ExportStatus.Used ? "attached" : "shared"
                : "local";
            views.Add(new DeviceView(
                d.BusId,
                d.VidPid,
                d.Product ?? "Unknown device",
                d.Manufacturer,
                SpeedLabel(d.Speed),
                state,
                state == "attached" && _attachedTo.TryGetValue(d.BusId, out var holder) ? holder.ClientId : null,
                state == "attached" && _attachedTo.TryGetValue(d.BusId, out var held) ? held.Since : null,
                _faults.GetValueOrDefault(d.BusId),
                d.Serial,
                d.Driver,
                [.. d.Interfaces.Select(i => InterfaceClassName(i.Class)).Distinct()]));
        }

        // A device the user shared that is gone stays visible, so they know it comes back shared. Only a bus ID
        // with a remembered VID:PID counts: one without never held a device Koppel saw, so there is nothing to show as gone.
        var remembered = _shared.Current.VidPids;
        views.AddRange(intent
            .Where(busId => remembered?.ContainsKey(busId) == true && devices.All(d => d.BusId != busId))
            .Select(busId => new DeviceView(busId, remembered![busId], _names.GetValueOrDefault(busId, "Unplugged device"), null, "", "offline", null, null, _faults.GetValueOrDefault(busId))));

        return new ServerDevices([.. views.OrderBy(v => v.BusId, StringComparer.Ordinal)], host.Unavailable);
    }

    /// <summary>USB interface classes as people know them.</summary>
    internal static string InterfaceClassName(byte code) => code switch
    {
        0x01 => "Audio",
        0x02 => "Communications",
        0x03 => "HID (keyboard, mouse, game pad)",
        0x05 => "Physical",
        0x06 => "Image",
        0x07 => "Printer",
        0x08 => "Mass storage",
        0x09 => "Hub",
        0x0A => "CDC data",
        0x0B => "Smart card",
        0x0D => "Content security",
        0x0E => "Video",
        0x0F => "Health",
        0x10 => "Audio/video",
        0xDC => "Diagnostic",
        0xE0 => "Wireless",
        0xEF => "Miscellaneous",
        0xFE => "Application specific",
        0xFF => "Vendor specific",
        _ => $"Class 0x{code:x2}",
    };

    internal static string SpeedLabel(UsbSpeed speed) => speed switch
    {
        UsbSpeed.Low => "USB 1.0 (1.5 Mbit/s)",
        UsbSpeed.Full => "USB 1.1 (12 Mbit/s)",
        UsbSpeed.High => "USB 2.0 (480 Mbit/s)",
        UsbSpeed.Super => "USB 3.0 (5 Gbit/s)",
        UsbSpeed.SuperPlus => "USB 3.1+ (10 Gbit/s)",
        _ => "Unknown speed",
    };

    [LoggerMessage(Level = LogLevel.Warning, Message = "Could not apply the share choice for {BusId}: {Reason}")]
    private partial void LogApplyFailed(string busId, string reason);

    [LoggerMessage(Level = LogLevel.Warning, Message = "Reading USB devices failed")]
    private partial void LogPollFailed(Exception exception);

    [LoggerMessage(Level = LogLevel.Warning, Message = "Setting up the shared devices at start-up failed; the next poll tries again")]
    private partial void LogStartUpFailed(Exception exception);

    [LoggerMessage(Level = LogLevel.Warning, Message = "Telling listeners about the device list failed")]
    private partial void LogDeliveryFailed(Exception exception);

    private sealed record Delivery(long Sequence, ServerDevices Devices, IReadOnlyList<PlugEvent> Plugs);

    private sealed record PlugEvent(string BusId, string Name, ActivityReason Reason, string Summary);
}
