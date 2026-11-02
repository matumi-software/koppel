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
    /// <summary>The TLS port for USB traffic. USB/IP's usual port, encrypted and behind auth.</summary>
    int ListenPort,
    /// <summary>False until the user finishes the first-run setup in the UI.</summary>
    bool SetupComplete = false)
{
    public static KoppelSettings Default { get; } = new(KoppelRole.Server, ShareOnStartup: true, ListenPort: 3240);

    /// <summary>The same rules a patch is held to, so a hand-edited file is checked like a REST change.</summary>
    public Dictionary<string, string[]> Validate() => new SettingsPatch(Role, ShareOnStartup, ListenPort, SetupComplete).Validate();
}

/// <summary>A partial change to <see cref="KoppelSettings"/>. Null means "leave as is".</summary>
internal sealed record SettingsPatch(
    KoppelRole? Role,
    bool? ShareOnStartup,
    int? ListenPort,
    bool? SetupComplete = null)
{
    public Dictionary<string, string[]> Validate()
    {
        var errors = new Dictionary<string, string[]>();
        if (ListenPort is < 1 or > 65535)
        {
            errors["listenPort"] = ["Use a port from 1 to 65535."];
        }

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
        ListenPort is { } port && port != current.ListenPort ? $"USB port set to {port}" : null,
        SetupComplete is true && !current.SetupComplete ? "Setup finished" : null,
    }.Where(part => part is not null));

    public KoppelSettings ApplyTo(KoppelSettings current) => current with
    {
        Role = Role ?? current.Role,
        ShareOnStartup = ShareOnStartup ?? current.ShareOnStartup,
        ListenPort = ListenPort ?? current.ListenPort,
        SetupComplete = SetupComplete ?? current.SetupComplete,
    };
}
