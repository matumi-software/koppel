using System.Runtime.Versioning;
using Microsoft.Extensions.Hosting.WindowsServices;
using Microsoft.Extensions.Options;

namespace Koppel.Tray;

/// <summary>The Windows service's name and the user-defined controls the tray icon sends it.</summary>
internal static class KoppelService
{
    public const string Name = "Koppel";

    /// <summary>Shut Koppel down. User-defined service controls run from 128 to 255.</summary>
    public const int ShutDownCommand = 200;

    /// <summary>Restart Koppel.</summary>
    public const int RestartCommand = 201;
}

/// <summary>
/// The Windows service lifetime, plus two user-defined controls the tray icon sends: <see cref="KoppelService.ShutDownCommand"/>
/// and <see cref="KoppelService.RestartCommand"/>. Windows gives interactive users the right to send a service its
/// user-defined controls but not to stop or start it, so the service stops itself. It is a clean stop (exit code 0), so the
/// recovery actions leave it stopped until the next boot or a manual start; for a restart, <see cref="ServiceRestart"/>
/// starts it again.
/// </summary>
[SupportedOSPlatform("windows")]
internal sealed partial class KoppelServiceLifetime : WindowsServiceLifetime
{
    private readonly IHostApplicationLifetime _applicationLifetime;
    private readonly ILogger _logger;

    public KoppelServiceLifetime(
        IHostEnvironment environment,
        IHostApplicationLifetime applicationLifetime,
        ILoggerFactory loggerFactory,
        IOptions<HostOptions> hostOptions,
        IOptions<WindowsServiceLifetimeOptions> serviceOptions)
        : base(environment, applicationLifetime, loggerFactory, hostOptions, serviceOptions)
    {
        _applicationLifetime = applicationLifetime;
        _logger = loggerFactory.CreateLogger<KoppelServiceLifetime>();
    }

    protected override void OnCustomCommand(int command)
    {
        if (command == KoppelService.ShutDownCommand)
        {
            LogShutDown(_logger);
            _applicationLifetime.StopApplication();
            return;
        }

        if (command == KoppelService.RestartCommand)
        {
            // Without the helper the service would only stop, so it stays up when the helper cannot be started.
            if (!ServiceRestart.Begin())
            {
                LogRestartFailed(_logger);
                return;
            }

            LogRestart(_logger);
            _applicationLifetime.StopApplication();
            return;
        }

        base.OnCustomCommand(command);
    }

    [LoggerMessage(Level = LogLevel.Information, Message = "Shutting down: asked to from the tray icon")]
    private static partial void LogShutDown(ILogger logger);

    [LoggerMessage(Level = LogLevel.Information, Message = "Restarting: asked to from the tray icon")]
    private static partial void LogRestart(ILogger logger);

    [LoggerMessage(Level = LogLevel.Warning, Message = "Could not restart as the tray icon asked: the helper that starts Koppel again did not start. Koppel keeps running.")]
    private static partial void LogRestartFailed(ILogger logger);
}
