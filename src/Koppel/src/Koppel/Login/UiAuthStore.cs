using System.Collections.Concurrent;
using System.Security.Cryptography;
using System.Text;
using Koppel.Core.Configuration;
using Koppel.Core.Security;
using Microsoft.Extensions.Hosting.Systemd;
using Microsoft.Extensions.Hosting.WindowsServices;

namespace Koppel.Login;

/// <summary>
/// The contents of the encrypted <c>ui-auth.json</c>: the UI's one password, hashed, the stamp that ends
/// every session when it changes, and the sessions that signed out with the moment each would have ended.
/// </summary>
internal sealed record UiCredentials(string? PasswordHash, string SecurityStamp, IReadOnlyDictionary<string, DateTimeOffset>? RevokedSessions = null)
{
    public static UiCredentials Empty { get; } = new(null, string.Empty);

    public static IReadOnlyDictionary<string, DateTimeOffset> NoSessions { get; } = new Dictionary<string, DateTimeOffset>();

    public bool HasPassword => PasswordHash is not null;

    /// <summary>Why a value is not acceptable, for the config file's validator.</summary>
    public static string? Invalid(UiCredentials value) =>
        value.PasswordHash is { } hash && !UiPassword.IsWellFormed(hash) ? "the password hash is not in the format this version writes" : null;
}

/// <summary>A new security stamp is in force. <see cref="KeepSession"/> is the one session that changed it and stays signed in.</summary>
internal sealed record UiStampChange(string Stamp, string? KeepSession);

internal enum SetupOutcome
{
    Done,
    AlreadySet,
    WrongCode,
}

/// <summary>
/// Owns <c>ui-auth.json</c> and the one-time setup code. While no password is set the code exists, in
/// memory, in <c>setup-code.txt</c> and, except under a Windows service or systemd, in the log; setting the password removes it, and clearing the
/// password (the reset command, or the file edited away) issues a new one. Same write-then-raise rule
/// as the other config files.
/// </summary>
internal sealed partial class UiAuthStore : IHostedService, IDisposable
{
    // No I, O, L, 0 or 1: nobody mistakes one for another when reading the code off a log.
    private const string CodeAlphabet = "ABCDEFGHJKMNPQRSTUVWXYZ23456789";
    private const int CodeLength = 8;

    private readonly JsonConfigFile<UiCredentials> _file;
    private readonly KoppelPaths _paths;
    private readonly ILogger<UiAuthStore> _logger;
    private readonly bool _logIsShared;
    private readonly SemaphoreSlim _setup = new(1, 1);
    private readonly Lock _codeGate = new();
    private string? _setupCode;
    private string _seenStamp = string.Empty;
    private int _passwordChecks;
    private readonly ConcurrentDictionary<string, string> _keepSessions = new();

    /// <param name="logIsShared">Whether the log is readable by users who cannot read the config folder. Left out, it is true under a Windows service or systemd; tests set it.</param>
    public UiAuthStore(KoppelPaths paths, SecretBox box, ILogger<UiAuthStore> logger, bool? logIsShared = null)
    {
        _paths = paths;
        _logger = logger;
        _logIsShared = logIsShared ?? (WindowsServiceHelpers.IsWindowsService() || SystemdHelpers.IsSystemdService());
        _file = new JsonConfigFile<UiCredentials>(paths.UiAuthFile, UiCredentials.Empty, KoppelJson.Files, logger, box.AsFileCodec(), UiCredentials.Invalid);
        _file.Changed += OnChangedAsync;
    }

    /// <summary>Raised when the stamp changes, so live hub connections signed in under the old one can be closed.</summary>
    public event Func<UiStampChange, Task>? StampChanged;

    public bool HasPassword => _file.Current.HasPassword;

    public string SecurityStamp => _file.Current.SecurityStamp;

    /// <summary>Sessions that signed out, each with when it would have ended anyway.</summary>
    public IReadOnlyDictionary<string, DateTimeOffset> RevokedSessions => _file.Current.RevokedSessions ?? UiCredentials.NoSessions;

    /// <summary>Writes the changed list of signed-out sessions. The password and stamp stay as they are.</summary>
    public Task UpdateRevokedSessionsAsync(Func<IReadOnlyDictionary<string, DateTimeOffset>, IReadOnlyDictionary<string, DateTimeOffset>> change, CancellationToken cancellationToken) =>
        _file.UpdateAsync(current => current with { RevokedSessions = change(current.RevokedSessions ?? UiCredentials.NoSessions) }, cancellationToken);

    /// <summary>The code that sets the first password, or null once there is one.</summary>
    public string? SetupCode
    {
        get
        {
            lock (_codeGate)
            {
                return _setupCode;
            }
        }
    }

    public async Task StartAsync(CancellationToken cancellationToken)
    {
        await _file.StartAsync(cancellationToken);
        _seenStamp = _file.Current.SecurityStamp;
        SyncSetupCode();
    }

    public Task StopAsync(CancellationToken cancellationToken)
    {
        _file.Stop();
        return Task.CompletedTask;
    }

    /// <summary>How many password hashes <see cref="VerifyPassword"/> has run. Tests use it to see how many guesses got as far as the hash.</summary>
    public int PasswordChecks => Volatile.Read(ref _passwordChecks);

    public bool VerifyPassword(string? password)
    {
        if (_file.Current.PasswordHash is not { } hash || string.IsNullOrEmpty(password) || password.Length > UiPassword.MaxLength)
        {
            return false;
        }

        Interlocked.Increment(ref _passwordChecks);
        return UiPassword.Verify(password, hash);
    }

    /// <summary>Sets the first password when the code is right. Works once: later calls find a password and are refused.</summary>
    public async Task<SetupOutcome> SetFirstPasswordAsync(string? code, string password, CancellationToken cancellationToken)
    {
        await _setup.WaitAsync(cancellationToken);
        try
        {
            if (HasPassword)
            {
                return SetupOutcome.AlreadySet;
            }

            if (!CodeMatches(code))
            {
                return SetupOutcome.WrongCode;
            }

            var hash = UiPassword.Hash(password);
            await _file.UpdateAsync(_ => new UiCredentials(hash, NewStamp()), cancellationToken);
            return SetupOutcome.Done;
        }
        finally
        {
            _setup.Release();
        }
    }

    /// <summary>Replaces the password and the stamp. Every session but <paramref name="keepSession"/> ends.</summary>
    public async Task ChangePasswordAsync(string next, string keepSession, CancellationToken cancellationToken)
    {
        var hash = UiPassword.Hash(next);
        await _file.UpdateAsync(current =>
        {
            // The session to keep travels with the stamp it belongs to, so two changes at once each keep their own.
            var stamp = NewStamp();
            _keepSessions[stamp] = keepSession;
            return current with { PasswordHash = hash, SecurityStamp = stamp };
        }, cancellationToken);
    }

    public void Dispose()
    {
        _file.Dispose();
        _setup.Dispose();
    }

    /// <summary>Clears the password and changes the stamp, as the reset command does. A file that cannot be read is replaced.</summary>
    public static async Task ResetFileAsync(KoppelPaths paths, SecretBox box, ILogger logger, CancellationToken cancellationToken)
    {
        using var file = new JsonConfigFile<UiCredentials>(paths.UiAuthFile, UiCredentials.Empty, KoppelJson.Files, logger, box.AsFileCodec(), UiCredentials.Invalid);
        try
        {
            await file.StartAsync(cancellationToken);
        }
        catch (ConfigFileException)
        {
            // What an admin who forgot the password and lost the keys is left with.
            File.Delete(paths.UiAuthFile);
            await file.StartAsync(cancellationToken);
        }

        await file.UpdateAsync(_ => new UiCredentials(null, NewStamp()), cancellationToken);
        file.Stop();
    }

    private static string NewStamp() => Guid.NewGuid().ToString("N");

    private Task OnChangedAsync(UiCredentials credentials)
    {
        SyncSetupCode();
        if (credentials.SecurityStamp == _seenStamp)
        {
            return Task.CompletedTask;
        }

        _seenStamp = credentials.SecurityStamp;
        _keepSessions.TryRemove(credentials.SecurityStamp, out var keep);
        return StampChanged.RaiseAsync(new UiStampChange(credentials.SecurityStamp, keep), _logger);
    }

    private bool CodeMatches(string? code)
    {
        var expected = SetupCode;
        if (expected is null || code is null)
        {
            return false;
        }

        var given = new string([.. code.Where(char.IsLetterOrDigit)]).ToUpperInvariant();
        return CryptographicOperations.FixedTimeEquals(Encoding.UTF8.GetBytes(given), Encoding.UTF8.GetBytes(expected));
    }

    // A password means no code and no code file; no password means a code, new if there was none.
    private void SyncSetupCode()
    {
        string? issued = null;
        lock (_codeGate)
        {
            if (HasPassword)
            {
                _setupCode = null;
            }
            else if (_setupCode is null)
            {
                _setupCode = issued = new string(RandomNumberGenerator.GetItems<char>(CodeAlphabet, CodeLength));
            }
        }

        if (issued is not null)
        {
            var shown = $"{issued[..4]}-{issued[4..]}";
            if (_logIsShared)
            {
                // A Windows service logs to the Application event log, which every interactive user can
                // read, and a systemd service logs to the journal, which members of adm and
                // systemd-journal can read. The code would let any of them set the first password, so the
                // log only says where the root-only or administrators-only file is. Docker is neither: its
                // log is readable only by whoever can run docker, so the code stays there.
                LogSetupCodeInFile(_paths.SetupCodeFile);
            }
            else
            {
                LogSetupCode(shown, _paths.SetupCodeFile);
            }

            WriteCodeFile(shown);
        }
        else if (HasPassword)
        {
            DeleteCodeFile();
        }
    }

    private void WriteCodeFile(string shown)
    {
        try
        {
            File.WriteAllText(_paths.SetupCodeFile, $"{shown}{Environment.NewLine}");
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
        {
            LogCodeFileFailed(_paths.SetupCodeFile, ex.Message);
        }
    }

    private void DeleteCodeFile()
    {
        try
        {
            File.Delete(_paths.SetupCodeFile);
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
        {
            LogCodeFileFailed(_paths.SetupCodeFile, ex.Message);
        }
    }

    [LoggerMessage(Level = LogLevel.Warning, Message = "Koppel setup code: {Code}. Enter it in the UI to set the admin password. It is also in {File} while no password is set.")]
    private partial void LogSetupCode(string code, string file);

    [LoggerMessage(Level = LogLevel.Warning, Message = "Koppel needs an admin password. The setup code to set it is in {File}, which only administrators and root can read.")]
    private partial void LogSetupCodeInFile(string file);

    [LoggerMessage(Level = LogLevel.Warning, Message = "Could not write or remove the setup code file {File} ({Reason}). The code in the log still works.")]
    private partial void LogCodeFileFailed(string file, string reason);
}
