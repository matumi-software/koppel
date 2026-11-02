using System.Security.Cryptography;
using System.Text.Json;
using Microsoft.Extensions.Logging;

namespace Koppel.Core.Configuration;

/// <summary>
/// One JSON config file that is the source of truth for a value. A change is written to disk first
/// (temp file, flush, move over the target); only a write that landed raises <see cref="Changed"/>.
/// Edits made to the file by hand are picked up by a watcher and raise the same event.
/// </summary>
/// <remarks>
/// <para>
/// This is the server half of the confirm rule: whoever listens to <see cref="Changed"/> pushes the
/// confirmed value to the UI, and the UI shows a change as done only when that push arrives.
/// </para>
/// <para>
/// A file that exists but cannot be used (unreadable, undecryptable, invalid JSON, failing the
/// validator) is never replaced by the defaults: <see cref="StartAsync"/> throws a
/// <see cref="ConfigFileException"/> and leaves it alone. Later, a bad hand edit is ignored, the last
/// good value stays and <see cref="Problem"/> says why.
/// </para>
/// <para>
/// <see cref="Changed"/> reaches subscribers in write order, one change at a time. A handler may await
/// an update of this same file: that nested change is delivered at once instead of waiting for the
/// handler that is running.
/// </para>
/// </remarks>
public sealed partial class JsonConfigFile<T> : IDisposable
    where T : class
{
    private const int ReadAttempts = 5;
    private const int ReplaceAttempts = 20;
    private const string TempSuffix = ".tmp";
    private static readonly TimeSpan WatchDebounce = TimeSpan.FromMilliseconds(250);
    private static readonly TimeSpan ReadRetryDelay = TimeSpan.FromMilliseconds(200);
    private static readonly TimeSpan ReplaceRetryDelay = TimeSpan.FromMilliseconds(100);

    private readonly string _path;
    private readonly T _defaults;
    private readonly JsonSerializerOptions _json;
    private readonly IFileCodec _codec;
    private readonly Func<T, string?>? _validate;
    private readonly ILogger _logger;
    private readonly SemaphoreSlim _gate = new(1, 1);
    private readonly AsyncLocal<bool> _delivering = new();
    private FileSystemWatcher? _watcher;
    private CancellationTokenSource? _pendingReload;
    private T _current;
    private string _currentJson;
    private (long Length, DateTime LastWrite)? _lastStamp;
    private Task _deliveryTail = Task.CompletedTask;
    private string? _problem;

    /// <param name="validate">Returns why a value is not acceptable, or null when it is.</param>
    public JsonConfigFile(string path, T defaults, JsonSerializerOptions json, ILogger logger, IFileCodec? codec = null, Func<T, string?>? validate = null)
    {
        _path = path;
        _defaults = defaults;
        _json = json;
        _logger = logger;
        _codec = codec ?? PlainTextCodec.Instance;
        _validate = validate;
        _current = defaults;
        _currentJson = JsonSerializer.Serialize(defaults, json);
    }

    /// <summary>Raised after a change is on disk, with the value as written.</summary>
    public event Func<T, Task>? Changed;

    public string Path => _path;

    public T Current => Volatile.Read(ref _current);

    /// <summary>
    /// Why the file on disk is not being applied (a hand edit that is unreadable or invalid), or null.
    /// <see cref="Current"/> is still the last good value.
    /// </summary>
    public string? Problem => Volatile.Read(ref _problem);

    /// <summary>
    /// Loads the file, writing the defaults if it does not exist yet, then starts watching it.
    /// </summary>
    /// <exception cref="ConfigFileException">The file exists but cannot be used. It is not touched.</exception>
    public async Task StartAsync(CancellationToken cancellationToken)
    {
        var directory = System.IO.Path.GetDirectoryName(_path)!;
        Directory.CreateDirectory(directory);
        DeleteStaleTempFiles(directory);
        if (File.Exists(_path))
        {
            SetCurrent(await LoadAsync(cancellationToken));
        }
        else
        {
            await WriteAtomicAsync(_defaults, cancellationToken);
            LogSeeded(_path);
        }

        _watcher = new FileSystemWatcher(directory, System.IO.Path.GetFileName(_path))
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

    /// <summary>
    /// Writes the changed value to disk, then raises <see cref="Changed"/>. A change that leaves the value
    /// as it was skips the write but still raises the event once, because whoever asked is waiting for the push.
    /// A hand edit made since the last read or write is loaded first, so the change does not undo it.
    /// </summary>
    public async Task<T> UpdateAsync(Func<T, T> change, CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(change);
        T next;
        Task previous;
        TaskCompletionSource mine;
        await _gate.WaitAsync(cancellationToken);
        try
        {
            await PickUpHandEditAsync(cancellationToken);
            next = change(Current);
            if (JsonSerializer.Serialize(next, _json) != _currentJson || !File.Exists(_path))
            {
                await WriteAtomicAsync(next, cancellationToken);
            }

            SetCurrent(next);
            (previous, mine) = TakeTicket();
        }
        finally
        {
            _gate.Release();
        }

        await DeliverAsync(next, previous, mine);
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
        Volatile.Write(ref _problem, null);
    }

    private (long Length, DateTime LastWrite)? Stamp()
    {
        var info = new FileInfo(_path);
        return info.Exists ? (info.Length, info.LastWriteTimeUtc) : null;
    }

    // Called under the gate. Loads the file when it changed behind our back and is usable.
    private async Task PickUpHandEditAsync(CancellationToken cancellationToken)
    {
        if (Stamp() is not { } stamp || stamp == _lastStamp)
        {
            return;
        }

        try
        {
            SetCurrent(await LoadAsync(cancellationToken));
        }
        catch (ConfigFileException ex)
        {
            LogReplacingBadEdit(_path, ex.Reason);
        }
    }

    // The ticket is taken under the gate, so deliveries are in the same order as the writes.
    private (Task Previous, TaskCompletionSource Mine) TakeTicket()
    {
        var mine = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var previous = _deliveryTail;
        _deliveryTail = mine.Task;
        return (previous, mine);
    }

    private async Task DeliverAsync(T value, Task previous, TaskCompletionSource mine)
    {
        try
        {
            if (!_delivering.Value)
            {
                await previous;
            }

            _delivering.Value = true;
            await Changed.RaiseAsync(value, _logger);
        }
        finally
        {
            mine.TrySetResult();
        }
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
            var previous = Task.CompletedTask;
            TaskCompletionSource? mine = null;
            await _gate.WaitAsync(cancellationToken);
            try
            {
                try
                {
                    var fromDisk = await LoadAsync(cancellationToken);

                    // Our own writes land here too; they serialize the same as Current and are ignored.
                    if (JsonSerializer.Serialize(fromDisk, _json) != _currentJson)
                    {
                        SetCurrent(fromDisk);
                        changed = fromDisk;
                        (previous, mine) = TakeTicket();
                    }
                    else
                    {
                        Volatile.Write(ref _problem, null);
                    }
                }
                catch (ConfigFileException ex)
                {
                    Volatile.Write(ref _problem, ex.Reason);
                    LogNotApplied(_path, ex.Reason);
                }
            }
            finally
            {
                _gate.Release();
            }

            if (changed is not null)
            {
                LogReloaded(_path);
                await DeliverAsync(changed, previous, mine!);
            }
        }
        catch (OperationCanceledException)
        {
            // A newer change superseded this one.
        }
        catch (ObjectDisposedException)
        {
            // Disposed while a reload was waiting.
        }
    }

    // Reads, decodes, parses and validates the file. Retries a file that is briefly locked.
    private async Task<T> LoadAsync(CancellationToken cancellationToken)
    {
        for (var attempt = 1; ; attempt++)
        {
            var stamp = Stamp();
            try
            {
                byte[] bytes;
                await using (var stream = new FileStream(_path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete))
                using (var buffer = new MemoryStream())
                {
                    await stream.CopyToAsync(buffer, cancellationToken);
                    bytes = buffer.ToArray();
                }

                var value = JsonSerializer.Deserialize<T>(_codec.Decode(bytes), _json)
                    ?? throw new ConfigFileException(_path, "it holds no value");
                if (_validate?.Invoke(value) is { } invalid)
                {
                    throw new ConfigFileException(_path, $"it is not valid ({invalid})");
                }

                _lastStamp = stamp;
                return value;
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException && attempt < ReadAttempts)
            {
                await Task.Delay(ReadRetryDelay, cancellationToken);
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
            {
                throw new ConfigFileException(_path, $"it could not be read ({ex.Message})", ex);
            }
            catch (JsonException ex)
            {
                throw new ConfigFileException(_path, $"it is not valid JSON for this file ({ex.Message})", ex);
            }
            catch (CryptographicException ex)
            {
                throw new ConfigFileException(_path, $"it could not be decrypted ({ex.Message})", ex);
            }
        }
    }

    private async Task WriteAtomicAsync(T value, CancellationToken cancellationToken)
    {
        var bytes = _codec.Encode(JsonSerializer.Serialize(value, _json));
        var temp = $"{_path}.{Guid.NewGuid():N}{TempSuffix}";
        try
        {
            await using (var stream = new FileStream(temp, FileMode.CreateNew, FileAccess.Write, FileShare.None))
            {
                await stream.WriteAsync(bytes, cancellationToken);
                stream.Flush(flushToDisk: true);
            }

            // On Windows a reader (the watcher, an editor, antivirus) can briefly hold the target open.
            for (var attempt = 1; ; attempt++)
            {
                try
                {
                    File.Move(temp, _path, overwrite: true);
                    break;
                }
                catch (Exception ex) when (ex is IOException or UnauthorizedAccessException && attempt < ReplaceAttempts)
                {
                    await Task.Delay(ReplaceRetryDelay, cancellationToken);
                }
            }
        }
        catch
        {
            TryDelete(temp);
            throw;
        }

        _lastStamp = Stamp();
    }

    // Leftovers of a crash between creating a temp file and moving it. Nothing else uses this pattern.
    private void DeleteStaleTempFiles(string directory)
    {
        var name = System.IO.Path.GetFileName(_path);
        foreach (var stale in Directory.EnumerateFiles(directory, $"{name}.*{TempSuffix}"))
        {
            TryDelete(stale);
        }
    }

    private static void TryDelete(string path)
    {
        try
        {
            File.Delete(path);
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
        {
            // Best effort: the next start sweeps it.
        }
    }

    [LoggerMessage(Level = LogLevel.Information, Message = "Wrote defaults to {Path}")]
    private partial void LogSeeded(string path);

    [LoggerMessage(Level = LogLevel.Information, Message = "Reloaded {Path} after an edit")]
    private partial void LogReloaded(string path);

    [LoggerMessage(Level = LogLevel.Warning, Message = "{Path} was not applied: {Reason}. Keeping the last good value")]
    private partial void LogNotApplied(string path, string reason);

    [LoggerMessage(Level = LogLevel.Warning, Message = "{Path} was edited by hand but is not usable ({Reason}); this update replaces it")]
    private partial void LogReplacingBadEdit(string path, string reason);
}
