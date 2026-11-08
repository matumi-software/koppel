using System.ComponentModel;
using System.IO.Pipes;
using System.Runtime.InteropServices;
using System.Runtime.Versioning;
using System.Text;
using Microsoft.Win32.SafeHandles;
using static Koppel.Tray.TrayNative;

namespace Koppel.Tray;

/// <summary>
/// Keeps a <see cref="TrayIcon"/> running in every active user session while the Windows service runs, the way Apollo
/// does: the service owns the icons, so they come and go with it. Each icon runs as that session's user, not as
/// LocalSystem, so the browser it opens is the user's. Its standard input is a pipe from here, which carries what the icon
/// shows (<see cref="TrayFeed"/>, <see cref="TrayWire"/>); disposing the pipe tells it to remove its icon and exit, and the
/// same happens if this process dies. A tray that fails never stops Koppel.
/// </summary>
[SupportedOSPlatform("windows")]
internal sealed partial class TrayLauncher(string uiUrl, TrayFeed feed, ILogger<TrayLauncher> logger) : BackgroundService
{
    private static readonly TimeSpan Interval = TimeSpan.FromSeconds(5);
    private static readonly TimeSpan QuickExit = TimeSpan.FromSeconds(30);
    private static readonly TimeSpan Backoff = TimeSpan.FromMinutes(1);
    private static readonly TimeSpan ExitWait = TimeSpan.FromSeconds(3);

    private readonly Dictionary<uint, Tray> _trays = [];
    private readonly Dictionary<uint, DateTimeOffset> _notBefore = [];

    /// <summary>Registers the launcher, with the UI address the icons open, and the feed of what they show.</summary>
    public static void Add(IServiceCollection services, string uiUrl)
    {
        services.AddSingleton<TrayFeed>();
        services.AddHostedService(sp => sp.GetRequiredService<TrayFeed>());
        services.AddHostedService(sp => new TrayLauncher(uiUrl, sp.GetRequiredService<TrayFeed>(), sp.GetRequiredService<ILogger<TrayLauncher>>()));
    }

    protected override async Task ExecuteAsync(CancellationToken stoppingToken)
    {
        try
        {
            while (!stoppingToken.IsCancellationRequested)
            {
                // Taken before the state is read, so a change while this round runs still wakes the next one.
                var changed = feed.WaitAsync();
                try
                {
                    Sync();
                    Publish();
                }
                catch (Exception exception) when (exception is not OutOfMemoryException)
                {
                    LogSyncFailed(logger, exception);
                }

                await Task.WhenAny(changed, Task.Delay(Interval, stoppingToken)).ConfigureAwait(false);
                stoppingToken.ThrowIfCancellationRequested();
            }
        }
        catch (OperationCanceledException) when (stoppingToken.IsCancellationRequested)
        {
        }
        finally
        {
            await CloseAllAsync().ConfigureAwait(false);
        }
    }

    /// <summary>Forgets trays that ended and starts one in each active session that has none.</summary>
    private void Sync()
    {
        var now = DateTimeOffset.UtcNow;
        foreach (var (session, tray) in _trays.ToList())
        {
            if (tray.HasExited)
            {
                _trays.Remove(session);
                tray.Dispose();
                if (now - tray.Started < QuickExit)
                {
                    // One that keeps ending at once is not started every few seconds.
                    _notBefore[session] = now + Backoff;
                    LogExitedQuickly(logger, session, Backoff.TotalMinutes);
                }
            }
        }

        foreach (var session in ActiveSessions())
        {
            if (_trays.ContainsKey(session) || (_notBefore.TryGetValue(session, out var notBefore) && now < notBefore))
            {
                continue;
            }

            _notBefore.Remove(session);
            if (Start(session) is { } tray)
            {
                _trays[session] = tray;
                LogStarted(logger, session);
            }
        }
    }

    /// <summary>Tells every tray the current state, when it differs from what that tray was last told.</summary>
    private void Publish()
    {
        var line = TrayWire.State(feed.Current);
        foreach (var tray in _trays.Values)
        {
            tray.Send(line);
        }
    }

    private static unsafe List<uint> ActiveSessions()
    {
        if (!WTSEnumerateSessionsW(0, 0, 1, out var sessions, out var count))
        {
            throw new Win32Exception(Marshal.GetLastPInvokeError());
        }

        try
        {
            var active = new List<uint>();
            for (var i = 0; i < count; i++)
            {
                // Session 0 is the services' own and has no desktop to show an icon on.
                if (sessions[i].State == WTSActive && sessions[i].SessionId != 0)
                {
                    active.Add(sessions[i].SessionId);
                }
            }

            return active;
        }
        finally
        {
            WTSFreeMemory(sessions);
        }
    }

    /// <summary>Starts the tray in <paramref name="session"/> as its user, or null when that cannot be done yet.</summary>
    private unsafe Tray? Start(uint session)
    {
        // Fails while the user is still signing in, and for a session with no user; the next round tries again.
        if (!WTSQueryUserToken(session, out var token))
        {
            var error = Marshal.GetLastPInvokeError();
            LogNoToken(logger, session, error);
            return null;
        }

        nint environment = 0;
        nint attributes = 0;
        // Room for many messages, so a tray that is slow to read never holds up this loop.
        var pipe = new AnonymousPipeServerStream(PipeDirection.Out, HandleInheritability.Inheritable, 64 * 1024);
        try
        {
            if (!CreateEnvironmentBlock(out environment, token, false))
            {
                throw new Win32Exception(Marshal.GetLastPInvokeError());
            }

            // The tray inherits the pipe's read end and nothing else. Without the handle list it would inherit every
            // inheritable handle this process has at that moment, such as the pipes of a tool another part of Koppel is running.
            nint size = 0;
            InitializeProcThreadAttributeList(0, 1, 0, ref size);
            attributes = Marshal.AllocHGlobal(size);
            if (!InitializeProcThreadAttributeList(attributes, 1, 0, ref size))
            {
                Marshal.FreeHGlobal(attributes);
                attributes = 0;
                throw new Win32Exception(Marshal.GetLastPInvokeError());
            }

            var readEnd = pipe.ClientSafePipeHandle.DangerousGetHandle();
            if (!UpdateProcThreadAttribute(attributes, 0, PROC_THREAD_ATTRIBUTE_HANDLE_LIST, &readEnd, sizeof(nint), 0, 0))
            {
                throw new Win32Exception(Marshal.GetLastPInvokeError());
            }

            var exe = Environment.ProcessPath ?? throw new InvalidOperationException("The service's own path is not known.");
            var commandLine = $"\"{exe}\" {TrayIcon.Argument} {uiUrl}\0".ToCharArray();
            fixed (char* desktop = "winsta0\\default")
            fixed (char* line = commandLine)
            {
                var startup = new STARTUPINFOEXW
                {
                    StartupInfo = new STARTUPINFOW
                    {
                        cb = (uint)sizeof(STARTUPINFOEXW),
                        lpDesktop = desktop,
                        dwFlags = STARTF_USESTDHANDLES,
                        hStdInput = readEnd,
                    },
                    lpAttributeList = attributes,
                };

                // CREATE_NO_WINDOW: Koppel.exe is a console program, and the tray must not open a console.
                const uint flags = CREATE_UNICODE_ENVIRONMENT | CREATE_NO_WINDOW | EXTENDED_STARTUPINFO_PRESENT;
                if (!CreateProcessAsUserW(token, exe, line, 0, 0, true, flags, environment, Path.GetDirectoryName(exe), &startup, out var process))
                {
                    throw new Win32Exception(Marshal.GetLastPInvokeError());
                }

                CloseHandle(process.hThread);
                pipe.DisposeLocalCopyOfClientHandle();
                var tray = new Tray(new SafeProcessHandle(process.hProcess, ownsHandle: true), pipe, DateTimeOffset.UtcNow);
                pipe = null;
                return tray;
            }
        }
        catch (Exception exception) when (exception is Win32Exception or InvalidOperationException)
        {
            LogStartFailed(logger, session, exception);
            _notBefore[session] = DateTimeOffset.UtcNow + Backoff;
            return null;
        }
        finally
        {
            pipe?.Dispose();
            if (attributes != 0)
            {
                DeleteProcThreadAttributeList(attributes);
                Marshal.FreeHGlobal(attributes);
            }

            if (environment != 0)
            {
                DestroyEnvironmentBlock(environment);
            }

            CloseHandle(token);
        }
    }

    /// <summary>Closes every pipe so each tray removes its icon, then ends any that have not gone after a few seconds.</summary>
    private async Task CloseAllAsync()
    {
        foreach (var tray in _trays.Values)
        {
            tray.ClosePipe();
        }

        var deadline = DateTimeOffset.UtcNow + ExitWait;
        while (_trays.Values.Any(tray => !tray.HasExited) && DateTimeOffset.UtcNow < deadline)
        {
            await Task.Delay(100, CancellationToken.None).ConfigureAwait(false);
        }

        foreach (var tray in _trays.Values)
        {
            if (!tray.HasExited)
            {
                tray.Kill();
            }

            tray.Dispose();
        }

        _trays.Clear();
    }

    private sealed class Tray(SafeProcessHandle process, AnonymousPipeServerStream pipe, DateTimeOffset started) : IDisposable
    {
        public DateTimeOffset Started { get; } = started;

        public bool HasExited => WaitForSingleObject(process, 0) != WAIT_TIMEOUT;

        /// <summary>What this tray was last told, so a state is sent once and not every round.</summary>
        private string? _sent;

        public void Send(string line)
        {
            if (line == _sent)
            {
                return;
            }

            try
            {
                pipe.Write(Encoding.UTF8.GetBytes(line + "\n"));
                pipe.Flush();
                _sent = line;
            }
            catch (IOException)
            {
                // The tray has gone; the next round notices it exited.
            }
        }

        public void ClosePipe() => pipe.Dispose();

        public void Kill() => TerminateProcess(process, 1);

        public void Dispose()
        {
            pipe.Dispose();
            process.Dispose();
        }
    }

    [LoggerMessage(Level = LogLevel.Information, Message = "Started the tray icon in session {Session}")]
    private static partial void LogStarted(ILogger logger, uint session);

    [LoggerMessage(Level = LogLevel.Debug, Message = "No user token for session {Session} yet (error {Error}); trying again shortly")]
    private static partial void LogNoToken(ILogger logger, uint session, int error);

    [LoggerMessage(Level = LogLevel.Warning, Message = "Could not start the tray icon in session {Session}; trying again in a minute")]
    private static partial void LogStartFailed(ILogger logger, uint session, Exception exception);

    [LoggerMessage(Level = LogLevel.Warning, Message = "The tray icon in session {Session} ended right after it started; trying again in {Minutes} min")]
    private static partial void LogExitedQuickly(ILogger logger, uint session, double minutes);

    [LoggerMessage(Level = LogLevel.Warning, Message = "Could not check the signed-in sessions for tray icons")]
    private static partial void LogSyncFailed(ILogger logger, Exception exception);
}
