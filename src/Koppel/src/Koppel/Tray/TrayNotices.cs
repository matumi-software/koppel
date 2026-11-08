using Koppel.Core.Activity;
using Koppel.Settings;

namespace Koppel.Tray;

/// <summary>A Windows notification the tray shows. <see cref="Page"/> is the UI page a click opens, such as <c>#clients</c>.</summary>
internal sealed record TrayNotice(string Title, string Text, string Page);

/// <summary>A pairing request as the notices need it: no code, no token.</summary>
internal sealed record TrayPairingRequest(string Id, string ClientName, string State);

/// <summary>
/// Decides which events become a notification, and remembers enough to say each thing once: a lost link once until it is
/// back, its return only after a loss was told, and each pairing request once. A failed attach is already recorded once
/// per reason, not on every retry (<see cref="Usb.UsbClient"/>). The settings turn each kind off. Not thread safe: the
/// feed calls it under its lock.
/// </summary>
internal sealed class TrayNoticeRules
{
    private readonly HashSet<string> _seenRequests = [];
    private bool _linkLost;

    public TrayNotice? FromActivity(ActivityEvent activity, KoppelSettings settings)
    {
        ArgumentNullException.ThrowIfNull(activity);
        ArgumentNullException.ThrowIfNull(settings);
        switch (activity.Reason)
        {
            case ActivityReason.LinkDown when activity.Outcome == ActivityOutcome.Failed:
                _linkLost = true;
                return settings.NotifyLink
                    ? new("Koppel lost the link to the server", $"{activity.Summary}. Koppel keeps trying, and attaches your devices again when it is back.", "#attach")
                    : null;
            case ActivityReason.LinkUp when activity.Outcome == ActivityOutcome.Succeeded && _linkLost:
                _linkLost = false;
                return settings.NotifyLink ? new("Koppel is linked again", $"{activity.Summary} again.", "#attach") : null;
            case ActivityReason.Attached when activity.Outcome == ActivityOutcome.Failed:
                return settings.NotifyAttachFailures
                    ? new("A device was not attached", activity.Detail is { Length: > 0 } reason ? $"{activity.Summary}: {reason}" : activity.Summary, "#attach")
                    : null;
            default:
                return null;
        }
    }

    /// <summary>A notice for each request that is waiting and was not told before.</summary>
    public IReadOnlyList<TrayNotice> FromPairing(IReadOnlyList<TrayPairingRequest> requests, KoppelSettings settings)
    {
        ArgumentNullException.ThrowIfNull(requests);
        ArgumentNullException.ThrowIfNull(settings);
        var notices = new List<TrayNotice>();
        foreach (var request in requests)
        {
            if (request.State == "waiting" && _seenRequests.Add(request.Id) && settings.NotifyPairing)
            {
                notices.Add(new("A client wants to pair", $"{request.ClientName} is waiting. Open Clients and type the code it shows.", "#clients"));
            }
        }

        // Forget the ones that went, so the set stays as small as the list.
        _seenRequests.IntersectWith(requests.Select(request => request.Id));
        return notices;
    }
}
