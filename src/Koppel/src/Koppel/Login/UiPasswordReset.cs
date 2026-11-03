using Koppel.Core.Security;
using Microsoft.Extensions.Logging.Abstractions;

namespace Koppel.Login;

/// <summary>
/// <c>Koppel --reset-password</c>: clears the UI password and changes the security stamp, then exits.
/// It edits the file on disk, so it works while Koppel runs (the service sees the change and opens setup
/// again at once) and when it does not (the next start opens it).
/// </summary>
internal static class UiPasswordReset
{
    public const string Flag = "--reset-password";

    public static bool Requested(string[] args) => args.Contains(Flag, StringComparer.OrdinalIgnoreCase);

    public static async Task<int> RunAsync(KoppelPaths paths, SecretBox box, TimeSpan setupWindow, TextWriter output)
    {
        try
        {
            await UiAuthStore.ResetFileAsync(paths, box, NullLogger.Instance, CancellationToken.None);
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
        {
            await output.WriteLineAsync($"Could not reset the password: {ex.Message}");
            await output.WriteLineAsync("Run this as an administrator (Windows), as root (Linux package: sudo koppel --reset-password) or as the user Koppel runs as (Docker: docker compose exec koppel dotnet Koppel.dll --reset-password).");
            return 1;
        }

        await output.WriteLineAsync("The UI password is cleared and every signed-in session is ended.");
        await output.WriteLineAsync($"Open the UI and set a new password within {(int)setupWindow.TotalMinutes} minutes: from now if Koppel is running, otherwise from when it starts. Whoever opens the UI first sets it.");
        return 0;
    }
}
