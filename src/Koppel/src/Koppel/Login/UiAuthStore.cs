using System.Collections.Concurrent;
using System.Globalization;
using Koppel.Core.Configuration;
using Koppel.Core.Security;

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
    Closed,
}

/// <summary>
/// Owns <c>ui-auth.json</c> and the first-run setup window. While no password is set, whoever reaches the
/// UI may set one, but only within <see cref="UiSetupWindow.Length"/> of Koppel starting without a
/// password, or of the password being cleared while it runs (the reset command, or the file edited away).
/// Setting the password closes the window. Same write-then-raise rule as the other config files.
/// </summary>
internal sealed partial class UiAuthStore : IHostedService, IDisposable
{
    private readonly JsonConfigFile<UiCredentials> _file;
    private readonly KoppelPaths _paths;
    private readonly UiSetupWindow _window;
    private readonly TimeProvider _time;
    private readonly ILogger<UiAuthStore> _logger;
    private readonly SemaphoreSlim _setup = new(1, 1);
    private readonly Lock _windowGate = new();
    private DateTimeOffset? _setupUntil;
    private string _seenStamp = string.Empty;
    private int _passwordChecks;
    private readonly ConcurrentDictionary<string, string> _keepSessions = new();

    public UiAuthStore(KoppelPaths paths, SecretBox box, UiSetupWindow window, TimeProvider time, ILogger<UiAuthStore> logger)
    {
        _paths = paths;
        _window = window;
        _time = time;
        _logger = logger;
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

    /// <summary>Until when the first password may be set, or null once there is one. It may lie in the past: setup has timed out.</summary>
    public DateTimeOffset? SetupUntil
    {
        get
        {
            lock (_windowGate)
            {
                return HasPassword ? null : _setupUntil;
            }
        }
    }

    /// <summary>True while no password is set and the setup window has not closed.</summary>
    public bool SetupOpen => SetupUntil is { } until && _time.GetUtcNow() < until;

    public async Task StartAsync(CancellationToken cancellationToken)
    {
        await _file.StartAsync(cancellationToken);
        _seenStamp = _file.Current.SecurityStamp;
        DeleteOldCodeFile();
        SyncWindow(reopen: false);
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

    /// <summary>Sets the first password while setup is open. Works once: later calls find a password and are refused.</summary>
    public async Task<SetupOutcome> SetFirstPasswordAsync(string password, CancellationToken cancellationToken)
    {
        await _setup.WaitAsync(cancellationToken);
        try
        {
            if (HasPassword)
            {
                return SetupOutcome.AlreadySet;
            }

            if (!SetupOpen)
            {
                return SetupOutcome.Closed;
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
        // No password under a new stamp is a reset, which opens setup again even if the window was still open.
        SyncWindow(reopen: credentials.SecurityStamp != _seenStamp);
        if (credentials.SecurityStamp == _seenStamp)
        {
            return Task.CompletedTask;
        }

        _seenStamp = credentials.SecurityStamp;
        _keepSessions.TryRemove(credentials.SecurityStamp, out var keep);
        return StampChanged.RaiseAsync(new UiStampChange(credentials.SecurityStamp, keep), _logger);
    }

    // A password closes the window. No password opens it when there was none open yet (a start, or the
    // password edited out of the file), or again when asked (a reset).
    private void SyncWindow(bool reopen)
    {
        DateTimeOffset? opened = null;
        lock (_windowGate)
        {
            if (HasPassword)
            {
                _setupUntil = null;
            }
            else if (_setupUntil is null || reopen)
            {
                _setupUntil = opened = _time.GetUtcNow() + _window.Length;
            }
        }

        if (opened is { } until)
        {
            var local = TimeZoneInfo.ConvertTime(until, _time.LocalTimeZone);
            LogWaitingForPassword(_window.Address, local.ToString("yyyy-MM-dd HH:mm:ss zzz", CultureInfo.InvariantCulture));
        }
    }

    // Earlier versions wrote a setup code here. It does nothing now, so it should not lie around.
    private void DeleteOldCodeFile()
    {
        try
        {
            File.Delete(_paths.OldSetupCodeFile);
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
        {
            LogOldCodeFileNotDeleted(_paths.OldSetupCodeFile, ex.Message);
        }
    }

    [LoggerMessage(Level = LogLevel.Warning, Message = "Koppel is waiting for its first password. Open {Address} and set it before {Until}. Whoever opens the UI first sets it. After that time, restart Koppel to set it.")]
    private partial void LogWaitingForPassword(string address, string until);

    [LoggerMessage(Level = LogLevel.Warning, Message = "Could not delete {File}, left by an older version ({Reason}). The setup code in it no longer does anything.")]
    private partial void LogOldCodeFileNotDeleted(string file, string reason);
}
