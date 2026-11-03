using System.Globalization;

namespace Koppel.Login;

/// <summary>
/// How long first-run setup stays open after Koppel starts without a password, from
/// <c>Ui:SetupWindowMinutes</c>, as Portainer does. <paramref name="Address"/> is where the log sends people.
/// </summary>
internal sealed record UiSetupWindow(TimeSpan Length, string Address)
{
    public const string Setting = "Ui:SetupWindowMinutes";

    public static readonly TimeSpan DefaultLength = TimeSpan.FromMinutes(10);

    /// <summary>Whole minutes, 1 or more. Unset, it is 10. Anything else stops the start, naming the setting.</summary>
    public static TimeSpan Resolve(string? setting)
    {
        if (string.IsNullOrWhiteSpace(setting))
        {
            return DefaultLength;
        }

        return int.TryParse(setting.Trim(), NumberStyles.None, CultureInfo.InvariantCulture, out var minutes) && minutes is >= 1 and <= 24 * 60
            ? TimeSpan.FromMinutes(minutes)
            : throw new InvalidOperationException($"{Setting} is \"{setting}\". Use a whole number of minutes from 1 to 1440.");
    }

    /// <summary>What the UI is told when the first password comes too late.</summary>
    public string ClosedReason =>
        $"Setup has timed out. Koppel takes the first password only in the first {(int)Length.TotalMinutes} minutes after it starts. Restart Koppel, then set it.";
}
