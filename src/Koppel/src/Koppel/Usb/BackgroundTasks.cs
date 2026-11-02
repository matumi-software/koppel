using System.Collections.Concurrent;

namespace Koppel.Usb;

/// <summary>
/// Runs fire-and-forget work so that nothing it throws goes unseen, and so a host that is stopping can
/// wait for it. An unhandled exception in a bare <c>Task.Run</c> is lost until the task is collected.
/// </summary>
internal sealed partial class BackgroundTasks(ILogger logger)
{
    private readonly ConcurrentDictionary<int, Task> _running = new();
    private int _next;

    /// <summary>Starts <paramref name="work"/>. Cancellation is its normal end; any other exception is logged with <paramref name="name"/>.</summary>
    public void Run(string name, Func<Task> work)
    {
        var id = Interlocked.Increment(ref _next);
        var done = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        _running[id] = done.Task;
        _ = Task.Run(async () =>
        {
            try
            {
                await work();
            }
            catch (OperationCanceledException)
            {
                // Stopped on purpose.
            }
            catch (Exception ex)
            {
                LogFailed(name, ex);
            }
            finally
            {
                _running.TryRemove(id, out _);
                done.SetResult();
            }
        }, CancellationToken.None);
    }

    /// <summary>Waits for what is running, up to <paramref name="timeout"/>.</summary>
    public async Task WaitAsync(TimeSpan timeout)
    {
        try
        {
            await Task.WhenAll(_running.Values).WaitAsync(timeout);
        }
        catch (TimeoutException)
        {
            LogStillRunning(_running.Count);
        }
    }

    [LoggerMessage(Level = LogLevel.Error, Message = "Background task {Name} failed")]
    private partial void LogFailed(string name, Exception exception);

    [LoggerMessage(Level = LogLevel.Warning, Message = "{Count} background tasks were still running when the host stopped")]
    private partial void LogStillRunning(int count);
}
