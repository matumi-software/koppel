using System.Runtime.Versioning;
using Microsoft.Extensions.Hosting.WindowsServices;
using Microsoft.Extensions.Options;

namespace Koppel.Tray;

/// <summary>The Windows service's name and the user-defined control the tray icon sends it.</summary>
internal static class KoppelService
{
    public const string Name = "Koppel";

    /// <summary>Shut Koppel down. User-defined service controls run from 128 to 255.</summary>
    public const int ShutDownCommand = 200;
}

/// <summary>
/// The Windows service lifetime, plus one user-defined control: <see cref="KoppelService.ShutDownCommand"/>, which the tray
/// icon sends to shut Koppel down. Windows gives interactive users the right to send a service its user-defined controls
/// but not to stop it, so the service stops itself. It is a clean stop (exit code 0), so the recovery actions leave it
/// stopped until the next boot or a manual start.
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

        base.OnCustomCommand(command);
    }

    [LoggerMessage(Level = LogLevel.Information, Message = "Shutting down: asked to from the tray icon")]
    private static partial void LogShutDown(ILogger logger);
}
