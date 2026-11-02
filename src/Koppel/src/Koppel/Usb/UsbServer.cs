using System.Collections.Concurrent;
using System.Text.Json;
using Koppel.Core.Configuration;
using Koppel.Core.Usb;
using Koppel.Settings;

namespace Koppel.Usb;

/// <summary>The bus IDs the user chose to share, kept in the plain <c>devices.json</c>.</summary>
internal sealed record SharedDevices(IReadOnlyList<string> BusIds)
{
    public static SharedDevices Empty { get; } = new([]);
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
    string? Fault);

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
    ILogger<UsbServer> logger) : BackgroundService
{
    private static readonly TimeSpan PollInterval = TimeSpan.FromSeconds(2);

    private readonly JsonConfigFile<SharedDevices> _shared = new(
        Path.Combine(paths.ConfigDirectory, "devices.json"), SharedDevices.Empty, KoppelJson.Files, logger);

    private readonly ConcurrentDictionary<string, string> _faults = new();
    private readonly ConcurrentDictionary<string, string> _attachedTo = new();
    private readonly SemaphoreSlim _gate = new(1, 1);
    private string _lastJson = "";
    private ServerDevices _current = new([], null);

    public event Func<ServerDevices, Task>? Changed;

    public ServerDevices Current => Volatile.Read(ref _current);

    /// <summary>Devices a client may import: shared and present.</summary>
    public IReadOnlyList<UsbDevice> Exportable() =>
        [.. host.ListDevices().Where(d => d.Driver == "usbip-host")];

    public IUsbHost Host => host;

    public override async Task StartAsync(CancellationToken cancellationToken)
    {
        await _shared.StartAsync(cancellationToken);
        if (host.Unavailable is null && settings.Current.ShareOnStartup)
        {
            foreach (var busId in _shared.Current.BusIds)
            {
                TryApply(busId, share: true);
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
        }, cancellationToken);
        TryApply(busId, share);
        await RefreshAsync(cancellationToken, force: true);
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
            _attachedTo[busId] = clientId;
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
                if (host.Unavailable is null)
                {
                    var present = host.ListDevices().ToDictionary(d => d.BusId);
                    foreach (var busId in _shared.Current.BusIds)
                    {
                        if (present.TryGetValue(busId, out var device) && device.Driver != "usbip-host" && !_faults.ContainsKey(busId))
                        {
                            TryApply(busId, share: true);
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
    }

    private ServerDevices Build()
    {
        var intent = _shared.Current.BusIds.ToHashSet(StringComparer.Ordinal);
        var devices = host.ListDevices();
        var views = devices.Select(d =>
        {
            var state = d.Driver == "usbip-host"
                ? host.GetStatus(d.BusId) == ExportStatus.Used ? "attached" : "shared"
                : "local";
            return new DeviceView(
                d.BusId,
                d.VidPid,
                d.Product ?? "Unknown device",
                d.Manufacturer,
                SpeedLabel(d.Speed),
                state,
                state == "attached" ? _attachedTo.GetValueOrDefault(d.BusId) : null,
                _faults.GetValueOrDefault(d.BusId));
        }).ToList();

        // Shared devices that are unplugged stay visible, so the user knows they come back shared.
        views.AddRange(intent
            .Where(busId => devices.All(d => d.BusId != busId))
            .Select(busId => new DeviceView(busId, "", "Unplugged device", null, "", "offline", null, _faults.GetValueOrDefault(busId))));

        return new ServerDevices([.. views.OrderBy(v => v.BusId, StringComparer.Ordinal)], host.Unavailable);
    }

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
