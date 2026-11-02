using System.Diagnostics;
using System.Globalization;

namespace Koppel.Core.Usb.Windows;

/// <summary>
/// The usbip-win2 driver (BSD-2-Clause, https://github.com/vadimgrn/usbip-win2), driven through its
/// documented <c>usbip.exe</c> command line. Its kernel driver connects to the USB/IP server itself.
/// </summary>
/// <param name="usbipExe">Koppel's bundled copy, or a standalone install's <c>C:\Program Files\USBip\usbip.exe</c>.</param>
public sealed class UsbipWin2Driver(string usbipExe) : IUsbIpDriver
{
    private static readonly TimeSpan CommandTimeout = TimeSpan.FromSeconds(30);

    public string? Unavailable => File.Exists(usbipExe)
        ? null
        : $"The Windows USB driver is not installed ({usbipExe} is missing). Reinstall Koppel to add it.";

    public async Task<int> AttachAsync(string host, int port, string busId, CancellationToken cancellationToken)
    {
        // --once: if the attach fails, fail now instead of retrying in the background.
        var output = await RunAsync(cancellationToken, "--tcp-port", Port(port), "attach", "--remote", host, "--bus-id", busId, "--terse", "--once");
        return int.TryParse(output.Trim(), NumberStyles.Integer, CultureInfo.InvariantCulture, out var driverPort)
            ? driverPort
            : throw new UsbHostException($"usbip.exe attached {busId} but did not say on which port: {output.Trim()}");
    }

    public async Task DetachAsync(string host, int port, string busId, int driverPort, CancellationToken cancellationToken)
    {
        // The driver attaches again by itself after a dropped connection, so stop that first.
        await RunAsync(cancellationToken, "--tcp-port", Port(port), "attach", "--remote", host, "--bus-id", busId, "--stop");
        await RunAsync(cancellationToken, "detach", "-p", driverPort.ToString(CultureInfo.InvariantCulture));
    }

    private static string Port(int port) => port.ToString(CultureInfo.InvariantCulture);

    private async Task<string> RunAsync(CancellationToken cancellationToken, params string[] arguments)
    {
        if (Unavailable is { } reason)
        {
            throw new UsbHostException(reason);
        }

        var start = new ProcessStartInfo(usbipExe)
        {
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            UseShellExecute = false,
            CreateNoWindow = true,
        };
        foreach (var argument in arguments)
        {
            start.ArgumentList.Add(argument);
        }

        using var process = Process.Start(start) ?? throw new UsbHostException($"Could not start {usbipExe}.");
        using var timeout = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(CommandTimeout);
        var stdout = process.StandardOutput.ReadToEndAsync(timeout.Token);
        var stderr = process.StandardError.ReadToEndAsync(timeout.Token);
        try
        {
            await process.WaitForExitAsync(timeout.Token);
        }
        catch (OperationCanceledException)
        {
            process.Kill(entireProcessTree: true);
            throw new UsbHostException($"usbip.exe {arguments[^1]} did not finish in {CommandTimeout.TotalSeconds:0} seconds.");
        }

        if (process.ExitCode != 0)
        {
            var error = (await stderr).Trim();
            throw new UsbHostException(error.StartsWith("error: ", StringComparison.Ordinal) ? error["error: ".Length..] : error);
        }

        return await stdout;
    }
}
