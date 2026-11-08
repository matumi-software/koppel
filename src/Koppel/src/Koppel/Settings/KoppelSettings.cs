namespace Koppel.Settings;

/// <summary>Which side of the link this machine plays.</summary>
public enum KoppelRole
{
    Server,
    Client,
    Both,
}

/// <summary>The settings stored in <c>settings.json</c>. The file is the source of truth.</summary>
public sealed record KoppelSettings(
    KoppelRole Role,
    /// <summary>Re-share the devices that were shared before a restart.</summary>
    bool ShareOnStartup,
    /// <summary>False until the user finishes the first-run setup in the UI.</summary>
    bool SetupComplete = false,
    /// <summary>Windows app: the tray icon says when a client is waiting to pair.</summary>
    bool NotifyPairing = true,
    /// <summary>Windows app: the tray icon says when the link to the server is lost, and when it is back.</summary>
    bool NotifyLink = true,
    /// <summary>Windows app: the tray icon says when a device could not be attached.</summary>
    bool NotifyAttachFailures = true)
{
    public static KoppelSettings Default { get; } = new(KoppelRole.Server, ShareOnStartup: true);

    /// <summary>The same rules a patch is held to, so a hand-edited file is checked like a REST change.</summary>
    public Dictionary<string, string[]> Validate() =>
        new SettingsPatch(Role, ShareOnStartup, SetupComplete, NotifyPairing, NotifyLink, NotifyAttachFailures).Validate();
}

/// <summary>A partial change to <see cref="KoppelSettings"/>. Null means "leave as is".</summary>
internal sealed record SettingsPatch(
    KoppelRole? Role,
    bool? ShareOnStartup,
    bool? SetupComplete = null,
    bool? NotifyPairing = null,
    bool? NotifyLink = null,
    bool? NotifyAttachFailures = null)
{
    public Dictionary<string, string[]> Validate()
    {
        var errors = new Dictionary<string, string[]>();
        if (Role is { } role && !Enum.IsDefined(role))
        {
            errors["role"] = ["Use server, client or both."];
        }

        return errors;
    }

    /// <summary>
    /// What this patch changes in <paramref name="current"/>, in words for the activity log, such as
    /// "Role set to client". Empty when it changes nothing.
    /// </summary>
    public string Describe(KoppelSettings current) => string.Join(". ", new[]
    {
        Role is { } role && role != current.Role ? $"Role set to {role.ToString().ToLowerInvariant()}" : null,
        ShareOnStartup is { } share && share != current.ShareOnStartup ? $"Share saved devices on startup turned {(share ? "on" : "off")}" : null,
        SetupComplete is true && !current.SetupComplete ? "Setup finished" : null,
        Turned("Notifications for pairing requests", NotifyPairing, current.NotifyPairing),
        Turned("Notifications for the link to the server", NotifyLink, current.NotifyLink),
        Turned("Notifications for failed attaches", NotifyAttachFailures, current.NotifyAttachFailures),
    }.Where(part => part is not null));

    public KoppelSettings ApplyTo(KoppelSettings current) => current with
    {
        Role = Role ?? current.Role,
        ShareOnStartup = ShareOnStartup ?? current.ShareOnStartup,
        SetupComplete = SetupComplete ?? current.SetupComplete,
        NotifyPairing = NotifyPairing ?? current.NotifyPairing,
        NotifyLink = NotifyLink ?? current.NotifyLink,
        NotifyAttachFailures = NotifyAttachFailures ?? current.NotifyAttachFailures,
    };

    private static string? Turned(string what, bool? next, bool current) =>
        next is { } value && value != current ? $"{what} turned {(value ? "on" : "off")}" : null;
}
