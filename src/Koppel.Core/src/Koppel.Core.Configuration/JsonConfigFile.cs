using System.Text.Json;
using Microsoft.Extensions.Logging;

namespace Koppel.Core.Configuration;

/// <summary>
/// One JSON config file that is the source of truth for a value. A change is written to disk first
/// (temp file, flush, move over the target); only a write that landed raises <see cref="Changed"/>.
/// Edits made to the file by hand are picked up by a watcher and raise the same event.
/// </summary>
/// <remarks>
/// This is the server half of the confirm rule: whoever listens to <see cref="Changed"/> pushes the
/// confirmed value to the UI, and the UI shows a change as done only when that push arrives.
/// </remarks>
public sealed partial class JsonConfigFile<T> : IDisposable
    where T : class
{
    private static readonly TimeSpan WatchDebounce = TimeSpan.FromMilliseconds(250);

    private readonly string _path;
    private readonly T _defaults;
    private readonly JsonSerializerOptions _json;
    private readonly IFileCodec _codec;
    private readonly ILogger _logger;
    private readonly SemaphoreSlim _gate = new(1, 1);
    private FileSystemWatcher? _watcher;
    private CancellationTokenSource? _pendingReload;
    private T _current;
    private string _currentJson;

    public JsonConfigFile(string path, T defaults, JsonSerializerOptions json, ILogger logger, IFileCodec? codec = null)
    {
        _path = path;
        _defaults = defaults;
        _json = json;
        _logger = logger;
        _codec = codec ?? PlainTextCodec.Instance;
        _current = defaults;
        _currentJson = JsonSerializer.Serialize(defaults, json);
    }

    /// <summary>Raised after a change is on disk, with the value as written.</summary>
    public event Func<T, Task>? Changed;

    public string Path => _path;

    public T Current => Volatile.Read(ref _current);

    /// <summary>Loads the file, writing the defaults if it does not exist yet, then starts watching it.</summary>
    public async Task StartAsync(CancellationToken cancellationToken)
    {
        Directory.CreateDirectory(System.IO.Path.GetDirectoryName(_path)!);
        if (File.Exists(_path))
        {
            if (await ReadFileAsync(cancellationToken) is { } loaded)
            {
                SetCurrent(loaded);
            }
        }
        else
        {
            await WriteAtomicAsync(_defaults, cancellationToken);
            LogSeeded(_path);
        }

        _watcher = new FileSystemWatcher(System.IO.Path.GetDirectoryName(_path)!, System.IO.Path.GetFileName(_path))
        {
            NotifyFilter = NotifyFilters.LastWrite | NotifyFilters.FileName | NotifyFilters.Size,
        };
        _watcher.Changed += (_, _) => ScheduleReload();
        _watcher.Created += (_, _) => ScheduleReload();
        _watcher.Renamed += (_, _) => ScheduleReload();
        _watcher.EnableRaisingEvents = true;
    }

    /// <summary>Stops watching. Safe to call more than once.</summary>
    public void Stop()
    {
        Interlocked.Exchange(ref _watcher, null)?.Dispose();
        var pending = Interlocked.Exchange(ref _pendingReload, null);
        pending?.Cancel();
        pending?.Dispose();
    }

    /// <summary>Writes the changed value to disk, then raises <see cref="Changed"/>.</summary>
    public async Task<T> UpdateAsync(Func<T, T> change, CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(change);
        T next;
        await _gate.WaitAsync(cancellationToken);
        try
        {
            next = change(Current);
            await WriteAtomicAsync(next, cancellationToken);
            SetCurrent(next);
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
        Stop();
        _gate.Dispose();
    }

    private void SetCurrent(T value)
    {
        Volatile.Write(ref _current, value);
        _currentJson = JsonSerializer.Serialize(value, _json);
    }

    private void ScheduleReload()
    {
        var cts = new CancellationTokenSource();
        var previous = Interlocked.Exchange(ref _pendingReload, cts);
        previous?.Cancel();
        previous?.Dispose();
        _ = ReloadAfterDebounceAsync(cts.Token);
    }

    private async Task ReloadAfterDebounceAsync(CancellationToken cancellationToken)
    {
        try
        {
            await Task.Delay(WatchDebounce, cancellationToken);
            T? changed = null;
            await _gate.WaitAsync(cancellationToken);
            try
            {
                // Our own writes land here too; they serialize the same as Current and are ignored.
                if (await ReadFileAsync(cancellationToken) is { } fromDisk
                    && JsonSerializer.Serialize(fromDisk, _json) != _currentJson)
                {
                    SetCurrent(fromDisk);
                    changed = fromDisk;
                }
            }
            finally
            {
                _gate.Release();
            }

            if (changed is not null)
            {
                LogReloaded(_path);
                await RaiseChangedAsync(changed);
            }
        }
        catch (OperationCanceledException)
        {
            // A newer change superseded this one.
        }
    }

    private async Task RaiseChangedAsync(T value)
    {
        if (Changed is { } handlers)
        {
            foreach (var handler in handlers.GetInvocationList().Cast<Func<T, Task>>())
            {
                await handler(value);
            }
        }
    }

    private async Task<T?> ReadFileAsync(CancellationToken cancellationToken)
    {
        try
        {
            byte[] bytes;
            await using (var stream = new FileStream(_path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete))
            using (var buffer = new MemoryStream())
            {
                await stream.CopyToAsync(buffer, cancellationToken);
                bytes = buffer.ToArray();
            }

            return JsonSerializer.Deserialize<T>(_codec.Decode(bytes), _json);
        }
        catch (Exception ex) when (ex is JsonException or IOException or System.Security.Cryptography.CryptographicException)
        {
            LogUnreadable(_path, ex);
            return null;
        }
    }

    private async Task WriteAtomicAsync(T value, CancellationToken cancellationToken)
    {
        var bytes = _codec.Encode(JsonSerializer.Serialize(value, _json));
        var temp = _path + ".tmp";
        await using (var stream = new FileStream(temp, FileMode.Create, FileAccess.Write, FileShare.None))
        {
            await stream.WriteAsync(bytes, cancellationToken);
            stream.Flush(flushToDisk: true);
        }

        // On Windows a reader (the watcher, an editor) can briefly hold the target open.
        for (var attempt = 1; ; attempt++)
        {
            try
            {
                File.Move(temp, _path, overwrite: true);
                return;
            }
            catch (IOException) when (attempt < 20)
            {
                await Task.Delay(100, cancellationToken);
            }
        }
    }

    [LoggerMessage(Level = LogLevel.Information, Message = "Wrote defaults to {Path}")]
    private partial void LogSeeded(string path);

    [LoggerMessage(Level = LogLevel.Information, Message = "Reloaded {Path} after an edit")]
    private partial void LogReloaded(string path);

    [LoggerMessage(Level = LogLevel.Warning, Message = "Could not read {Path}; keeping the last good value")]
    private partial void LogUnreadable(string path, Exception exception);
}
