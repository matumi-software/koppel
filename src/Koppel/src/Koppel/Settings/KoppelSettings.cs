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
    bool ShareOnStartup,
    bool RequireTls,
    int ListenPort)
{
    public static KoppelSettings Default { get; } = new(KoppelRole.Server, ShareOnStartup: false, RequireTls: true, ListenPort: 3240);
}

/// <summary>A partial change to <see cref="KoppelSettings"/>. Null means "leave as is".</summary>
internal sealed record SettingsPatch(
    KoppelRole? Role,
    bool? ShareOnStartup,
    bool? RequireTls,
    int? ListenPort)
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

    public KoppelSettings ApplyTo(KoppelSettings current) => current with
    {
        Role = Role ?? current.Role,
        ShareOnStartup = ShareOnStartup ?? current.ShareOnStartup,
        RequireTls = RequireTls ?? current.RequireTls,
        ListenPort = ListenPort ?? current.ListenPort,
    };
}
