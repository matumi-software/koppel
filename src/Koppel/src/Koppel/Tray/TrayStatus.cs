using System.Text.Json;
using Koppel.Settings;
using Koppel.Usb;

namespace Koppel.Tray;

/// <summary>What the tray icon shows: its tooltip, and whether something needs the user.</summary>
internal sealed record TrayState(string Tip, bool Attention)
{
    /// <summary>Before the service has said anything, and for a message the tray does not understand: the plain icon.</summary>
    public static TrayState Plain { get; } = new("Koppel", false);
}

/// <summary>What the tray's state is made from: names and counts only, nothing secret.</summary>
/// <param name="ServerHost">The host of the server a client is linked to, as the UI shows it, or null.</param>
internal sealed record TrayFacts(KoppelRole Role, bool PasswordSet, bool SetupOpen, ClientDevices Client, string? ServerHost, ServerDevices Server);

/// <summary>Turns <see cref="TrayFacts"/> into the tray's tooltip. Kept apart from Win32 so it is tested on every platform.</summary>
internal static class TrayStatus
{
    /// <summary>A notification icon's tooltip holds 127 characters.</summary>
    public const int MaxTip = 127;

    public static TrayState Describe(TrayFacts facts)
    {
        if (!facts.PasswordSet)
        {
            return facts.SetupOpen
                ? new("Koppel: setting up. Open Koppel to set the password.", true)
                : new("Koppel: setup has timed out. Restart Koppel to set it up.", true);
        }

        var lines = new List<(string Text, bool Attention)>();
        if (facts.Role is KoppelRole.Server or KoppelRole.Both)
        {
            lines.Add(ServerLine(facts.Server));
        }

        if (facts.Role is KoppelRole.Client or KoppelRole.Both)
        {
            lines.Add(ClientLine(facts.Client, facts.ServerHost));
        }

        var tip = lines.Count == 1 ? $"Koppel: {lines[0].Text}" : "Koppel\n" + string.Join("\n", lines.Select(line => Capitalise(line.Text)));
        return new(Fit(tip), lines.Any(line => line.Attention));
    }

    private static (string, bool) ServerLine(ServerDevices server)
    {
        if (server.Unavailable is not null)
        {
            return ("server, sharing is not available", true);
        }

        var shared = server.Devices.Count(d => d.State is "shared" or "attached" or "offline");
        var inUse = server.Devices.Count(d => d.State == "attached");
        return ($"server, {Count(shared, "device")} shared, {inUse} in use", false);
    }

    private static (string, bool) ClientLine(ClientDevices client, string? serverHost)
    {
        var server = serverHost ?? "the server";
        var attached = client.Devices.Count(d => d.AttachedHere);
        return client.Link switch
        {
            "connected" => ($"linked to {server}, {Count(attached, "device")} attached", false),
            "connecting" => ($"connecting to {server}", false),
            "unconfigured" => ("no server yet", false),
            _ => ($"cannot reach {server}", true),
        };
    }

    private static string Count(int count, string noun) => count == 1 ? $"1 {noun}" : $"{count} {noun}s";

    private static string Capitalise(string text) => text.Length == 0 ? text : char.ToUpperInvariant(text[0]) + text[1..];

    private static string Fit(string tip) => tip.Length <= MaxTip ? tip : tip[..(MaxTip - 3)] + "...";
}

/// <summary>
/// The lines the service writes to each tray's standard input: one JSON object per line, with a <c>type</c>. A tray skips a
/// line it cannot read or a type it does not know, so a newer service can add messages without breaking an older tray.
/// </summary>
internal static class TrayWire
{
    public static string State(TrayState state) =>
        JsonSerializer.Serialize(new { type = "state", tip = state.Tip, attention = state.Attention });

    /// <summary>The state in <paramref name="line"/>, or null when it is not a state message this tray can read.</summary>
    public static TrayState? ReadState(string line)
    {
        try
        {
            using var json = JsonDocument.Parse(line);
            var root = json.RootElement;
            if (root.ValueKind != JsonValueKind.Object
                || !root.TryGetProperty("type", out var type) || type.ValueKind != JsonValueKind.String || type.GetString() != "state"
                || !root.TryGetProperty("tip", out var tip) || tip.ValueKind != JsonValueKind.String)
            {
                return null;
            }

            var attention = root.TryGetProperty("attention", out var flag) && flag.ValueKind == JsonValueKind.True;
            var text = tip.GetString()!;
            return new(text.Length <= TrayStatus.MaxTip ? text : text[..TrayStatus.MaxTip], attention);
        }
        catch (JsonException)
        {
            return null;
        }
    }

    /// <summary>A notification balloon's title holds 63 characters and its text 255.</summary>
    public const int MaxTitle = 63;
    public const int MaxText = 255;

    public static string Notice(TrayNotice notice) =>
        JsonSerializer.Serialize(new { type = "notice", title = notice.Title, text = notice.Text, page = notice.Page });

    /// <summary>
    /// The notice in <paramref name="line"/>, or null when it is not one this tray can read. The page must be one of the UI's
    /// own (<c>#</c> and letters), so a line can never send the browser anywhere else.
    /// </summary>
    public static TrayNotice? ReadNotice(string line)
    {
        try
        {
            using var json = JsonDocument.Parse(line);
            var root = json.RootElement;
            if (root.ValueKind != JsonValueKind.Object
                || !root.TryGetProperty("type", out var type) || type.ValueKind != JsonValueKind.String || type.GetString() != "notice"
                || !root.TryGetProperty("title", out var title) || title.ValueKind != JsonValueKind.String
                || !root.TryGetProperty("text", out var text) || text.ValueKind != JsonValueKind.String)
            {
                return null;
            }

            var page = root.TryGetProperty("page", out var target) && target.ValueKind == JsonValueKind.String
                && target.GetString() is { Length: > 1 and <= 32 } value && value[0] == '#' && value[1..].All(char.IsAsciiLetterLower)
                ? value
                : "";
            return new(Cut(title.GetString()!, MaxTitle), Cut(text.GetString()!, MaxText), page);
        }
        catch (JsonException)
        {
            return null;
        }
    }

    private static string Cut(string value, int max) => value.Length <= max ? value : value[..(max - 3)] + "...";
}
