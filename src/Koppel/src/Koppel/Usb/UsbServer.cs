using System.Collections.Concurrent;
using System.Text.Json;
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
/// kernel, and raises <see cref="Changed"/> whenever what the kernel reports changes. Polls sysfs
/// every two seconds, which also catches plugging and unplugging.
/// </summary>
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
        Path.Combine(paths.ConfigDirectory, "devices.json"), SharedDevices.Empty, KoppelJson.Files, logger);

    private readonly ConcurrentDictionary<string, string> _faults = new();
    private readonly ConcurrentDictionary<string, (string ClientId, DateTimeOffset Since)> _attachedTo = new();
    private readonly SemaphoreSlim _gate = new(1, 1);
    private readonly ConcurrentDictionary<string, string> _names = new();
    private HashSet<string>? _present;
    private string _lastJson = "";
    private ServerDevices _current = new([], null);

    public event Func<ServerDevices, Task>? Changed;

    public ServerDevices Current => Volatile.Read(ref _current);

    /// <summary>Devices a client may import: shared and present.</summary>
    public IReadOnlyList<UsbDevice> Exportable() =>
        [.. host.ListDevices().Where(host.IsShared)];

    public IUsbHost Host => host;

    public override async Task StartAsync(CancellationToken cancellationToken)
    {
        await _shared.StartAsync(cancellationToken);
        if (host.Unavailable is null && settings.Current.ShareOnStartup && settings.Current.Role != KoppelRole.Client)
        {
            foreach (var device in host.ListDevices().Where(_shared.Current.Expects))
            {
                TryApply(device.BusId, share: true);
            }
        }

        await RefreshAsync(cancellationToken);
        await base.StartAsync(cancellationToken);
    }

    public override async Task StopAsync(CancellationToken cancellationToken)
    {
        _shared.Stop();
        await base.StopAsync(cancellationToken);
    }

    /// <summary>
    /// Records the choice in <c>devices.json</c>, applies it to the kernel, then raises
    /// <see cref="Changed"/> with what the kernel now reports. A kernel refusal shows as the device's fault.
    /// </summary>
    public async Task SetSharedAsync(string busId, bool share, CancellationToken cancellationToken)
    {
        await _shared.UpdateAsync(current => current with
        {
            BusIds = share ? [.. current.BusIds.Append(busId).Distinct()] : [.. current.BusIds.Where(b => b != busId)],
            VidPids = share && host.ListDevices().FirstOrDefault(d => d.BusId == busId) is { } device
                ? new Dictionary<string, string>(current.VidPids ?? new Dictionary<string, string>()) { [busId] = device.VidPid }
                : current.VidPids?.Where(p => p.Key != busId).ToDictionary(p => p.Key, p => p.Value),
        }, cancellationToken);
        TryApply(busId, share);
        await RefreshAsync(cancellationToken, force: true);

        var name = _names.GetValueOrDefault(busId, busId);
        var entity = new EntityRef(busId, name, "device");
        if (_faults.TryGetValue(busId, out var fault))
        {
            await activity.RecordAsync(entity, share ? ActivityReason.Shared : ActivityReason.Unshared, ActivityOutcome.Failed, $"{name} was not {(share ? "shared" : "unshared")}", fault);
        }
        else
        {
            await activity.RecordAsync(entity, share ? ActivityReason.Shared : ActivityReason.Unshared, ActivityOutcome.Succeeded, share ? $"{name} is shared" : $"{name} is no longer shared");
        }
    }

    /// <summary>Called by the data plane when a client imports or releases a device.</summary>
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

        return RefreshAsync(CancellationToken.None, force: true);
    }

    public override void Dispose()
    {
        _shared.Dispose();
        _gate.Dispose();
        base.Dispose();
    }

    protected override async Task ExecuteAsync(CancellationToken stoppingToken)
    {
        using var timer = new PeriodicTimer(PollInterval);
        while (await timer.WaitForNextTickAsync(stoppingToken))
        {
            try
            {
                // A shared device that comes back after unplugging needs binding again.
                // Only a server re-shares. A pure client never binds anything to usbip-host.
                if (host.Unavailable is null && settings.Current.Role != KoppelRole.Client)
                {
                    foreach (var device in host.ListDevices().Where(_shared.Current.Expects))
                    {
                        if (!host.IsShared(device) && !_faults.ContainsKey(device.BusId))
                        {
                            TryApply(device.BusId, share: true);
                        }
                    }
                }

                await RefreshAsync(stoppingToken);
            }
            catch (Exception ex) when (ex is IOException or UsbHostException)
            {
                LogPollFailed(ex);
            }
        }
    }

    private void TryApply(string busId, bool share)
    {
        try
        {
            if (share)
            {
                host.Share(busId);
            }
            else
            {
                host.Unshare(busId);
            }

            _faults.TryRemove(busId, out _);
        }
        catch (UsbHostException ex)
        {
            _faults[busId] = ex.Message;
            LogApplyFailed(busId, ex.Message);
        }
    }

    private async Task RefreshAsync(CancellationToken cancellationToken, bool force = false)
    {
        ServerDevices next;
        await _gate.WaitAsync(cancellationToken);
        try
        {
            next = Build();
            var json = JsonSerializer.Serialize(next, KoppelJson.Files);
            if (!force && json == _lastJson)
            {
                return;
            }

            _lastJson = json;
            Volatile.Write(ref _current, next);
        }
        finally
        {
            _gate.Release();
        }

        if (Changed is { } handler)
        {
            await handler(next);
        }

        await RecordPlugChangesAsync(next);
    }

    /// <summary>Logs devices that appeared or disappeared since the last look. The first look only remembers.</summary>
    private async Task RecordPlugChangesAsync(ServerDevices devices)
    {
        var present = devices.Devices.Where(d => d.State != "offline").Select(d => d.BusId).ToHashSet(StringComparer.Ordinal);
        var previous = Interlocked.Exchange(ref _present, present);
        if (previous is null)
        {
            return;
        }

        foreach (var busId in present.Except(previous))
        {
            var name = _names.GetValueOrDefault(busId, busId);
            await activity.RecordAsync(new EntityRef(busId, name, "device"), ActivityReason.Plugged, ActivityOutcome.Succeeded, $"{name} was plugged in on bus {busId}");
        }

        foreach (var busId in previous.Except(present))
        {
            var name = _names.GetValueOrDefault(busId, busId);
            await activity.RecordAsync(new EntityRef(busId, name, "device"), ActivityReason.Unplugged, ActivityOutcome.Succeeded, $"{name} was unplugged from bus {busId}");
        }
    }

    private ServerDevices Build()
    {
        var intent = _shared.Current.BusIds.ToHashSet(StringComparer.Ordinal);
        var devices = host.ListDevices();
        var views = devices.Select(d =>
        {
            _names[d.BusId] = d.Product ?? "Unknown device";
            var state = host.IsShared(d)
                ? host.GetStatus(d.BusId) == ExportStatus.Used ? "attached" : "shared"
                : "local";
            return new DeviceView(
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
                [.. d.Interfaces.Select(i => InterfaceClassName(i.Class)).Distinct()]);
        }).ToList();

        // Shared devices that are unplugged stay visible, so the user knows they come back shared.
        views.AddRange(intent
            .Where(busId => devices.All(d => d.BusId != busId))
            .Select(busId => new DeviceView(busId, "", _names.GetValueOrDefault(busId, "Unplugged device"), null, "", "offline", null, null, _faults.GetValueOrDefault(busId))));

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
}
