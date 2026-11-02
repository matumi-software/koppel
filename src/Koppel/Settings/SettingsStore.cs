using System.Text.Json;
using System.Text.Json.Serialization;

namespace Koppel.Settings;

/// <summary>
/// Owns <c>settings.json</c>. A change is applied by writing the file first; only a write that
/// landed raises <see cref="Changed"/>, which is what the UI waits for. Hand edits to the file are
/// picked up by a watcher and raise the same event.
/// </summary>
internal sealed partial class SettingsStore(KoppelPaths paths, ILogger<SettingsStore> logger) : IHostedService, IDisposable
{
    internal static readonly JsonSerializerOptions JsonOptions = new(JsonSerializerDefaults.Web)
    {
        WriteIndented = true,
        Converters = { new JsonStringEnumConverter(JsonNamingPolicy.CamelCase) },
    };

    private static readonly TimeSpan WatchDebounce = TimeSpan.FromMilliseconds(250);

    private readonly SemaphoreSlim _gate = new(1, 1);
    private FileSystemWatcher? _watcher;
    private CancellationTokenSource? _pendingReload;
    private KoppelSettings _current = KoppelSettings.Default;

    /// <summary>Raised after a change is on disk, with the settings as written.</summary>
    public event Func<KoppelSettings, Task>? Changed;

    public KoppelSettings Current => Volatile.Read(ref _current);

    public async Task StartAsync(CancellationToken cancellationToken)
    {
        Directory.CreateDirectory(paths.ConfigDirectory);
        if (File.Exists(paths.SettingsFile))
        {
            _current = await ReadFileAsync(cancellationToken) ?? KoppelSettings.Default;
        }
        else
        {
            await WriteAtomicAsync(KoppelSettings.Default, cancellationToken);
            LogSeeded(paths.SettingsFile);
        }

        _watcher = new FileSystemWatcher(paths.ConfigDirectory, Path.GetFileName(paths.SettingsFile))
        {
            NotifyFilter = NotifyFilters.LastWrite | NotifyFilters.FileName | NotifyFilters.Size,
        };
        _watcher.Changed += (_, _) => ScheduleReload();
        _watcher.Created += (_, _) => ScheduleReload();
        _watcher.Renamed += (_, _) => ScheduleReload();
        _watcher.EnableRaisingEvents = true;
    }

    public Task StopAsync(CancellationToken cancellationToken)
    {
        _watcher?.Dispose();
        _watcher = null;
        _pendingReload?.Cancel();
        return Task.CompletedTask;
    }

    /// <summary>Writes the patched settings to disk, then raises <see cref="Changed"/>.</summary>
    public async Task<KoppelSettings> ApplyAsync(SettingsPatch patch, CancellationToken cancellationToken)
    {
        KoppelSettings next;
        await _gate.WaitAsync(cancellationToken);
        try
        {
            next = patch.ApplyTo(Current);
            await WriteAtomicAsync(next, cancellationToken);
            Volatile.Write(ref _current, next);
        }
        finally
        {
            _gate.Release();
        }

        await RaiseChangedAsync(next);
        return next;
    }

    public void Dispose()
    {
        _watcher?.Dispose();
        _pendingReload?.Dispose();
        _gate.Dispose();
    }

    private void ScheduleReload()
    {
        var cts = new CancellationTokenSource();
        Interlocked.Exchange(ref _pendingReload, cts)?.Cancel();
        _ = ReloadAfterDebounceAsync(cts.Token);
    }

    private async Task ReloadAfterDebounceAsync(CancellationToken cancellationToken)
    {
        try
        {
            await Task.Delay(WatchDebounce, cancellationToken);
            KoppelSettings? changed = null;
            await _gate.WaitAsync(cancellationToken);
            try
            {
                var fromDisk = await ReadFileAsync(cancellationToken);
                // Our own writes land here too; they match Current and are ignored.
                if (fromDisk is not null && fromDisk != Current)
                {
                    Volatile.Write(ref _current, fromDisk);
                    changed = fromDisk;
                }
            }
            finally
            {
                _gate.Release();
            }

            if (changed is not null)
            {
                LogReloaded(paths.SettingsFile);
                await RaiseChangedAsync(changed);
            }
        }
        catch (OperationCanceledException)
        {
            // A newer change superseded this one.
        }
    }

    private async Task RaiseChangedAsync(KoppelSettings settings)
    {
        if (Changed is { } handlers)
        {
            foreach (var handler in handlers.GetInvocationList().Cast<Func<KoppelSettings, Task>>())
            {
                await handler(settings);
            }
        }
    }

    private async Task<KoppelSettings?> ReadFileAsync(CancellationToken cancellationToken)
    {
        try
        {
            await using var stream = new FileStream(paths.SettingsFile, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete);
            return await JsonSerializer.DeserializeAsync<KoppelSettings>(stream, JsonOptions, cancellationToken);
        }
        catch (Exception ex) when (ex is JsonException or IOException)
        {
            LogUnreadable(paths.SettingsFile, ex);
            return null;
        }
    }

    /// <summary>Temp file, flush to disk, then move over the target, so readers never see half a file.</summary>
    private async Task WriteAtomicAsync(KoppelSettings settings, CancellationToken cancellationToken)
    {
        var temp = paths.SettingsFile + ".tmp";
        await using (var stream = new FileStream(temp, FileMode.Create, FileAccess.Write, FileShare.None))
        {
            await JsonSerializer.SerializeAsync(stream, settings, JsonOptions, cancellationToken);
            await stream.FlushAsync(cancellationToken);
            stream.Flush(flushToDisk: true);
        }

        // On Windows a reader (the watcher, an editor) can briefly hold the target open.
        for (var attempt = 1; ; attempt++)
        {
            try
            {
                File.Move(temp, paths.SettingsFile, overwrite: true);
                return;
            }
            catch (IOException) when (attempt < 20)
            {
                await Task.Delay(100, cancellationToken);
            }
        }
    }

    [LoggerMessage(Level = LogLevel.Information, Message = "Wrote default settings to {Path}")]
    private partial void LogSeeded(string path);

    [LoggerMessage(Level = LogLevel.Information, Message = "Reloaded settings after an edit to {Path}")]
    private partial void LogReloaded(string path);

    [LoggerMessage(Level = LogLevel.Warning, Message = "Could not read settings from {Path}; keeping the last good settings")]
    private partial void LogUnreadable(string path, Exception exception);
}
