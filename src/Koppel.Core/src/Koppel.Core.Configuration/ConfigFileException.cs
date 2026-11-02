namespace Koppel.Core.Configuration;

/// <summary>
/// A config file exists but cannot be used: it is unreadable, cannot be decrypted, is not valid JSON or
/// fails validation. The file is left exactly as it was, so the cause can be fixed and Koppel restarted.
/// </summary>
public sealed class ConfigFileException : Exception
{
    public ConfigFileException(string path, string reason, Exception? inner = null)
        : base($"Koppel cannot use {path}: {reason}. The file was not changed. If it is encrypted, the key ring that encrypted it is missing or has changed: restore the keys directory. If it was edited by hand, fix the edit or restore the file. Then start Koppel again.", inner)
    {
        Path = path;
        Reason = reason;
    }

    /// <summary>The file that could not be used.</summary>
    public string Path { get; }

    /// <summary>What is wrong with it, in one line.</summary>
    public string Reason { get; }
}
