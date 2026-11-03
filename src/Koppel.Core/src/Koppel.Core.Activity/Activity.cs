using Koppel.Core.Configuration;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Http;
using Microsoft.AspNetCore.Routing;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Logging.Abstractions;

namespace Koppel.Core.Activity;

/// <summary>What an activity event is about: a device, a client, this machine or a server link.</summary>
/// <param name="Type"><c>device</c>, <c>client</c>, <c>machine</c> or <c>link</c>.</param>
public sealed record EntityRef(string Id, string Name, string Type);

/// <summary>Why an event was recorded. Sent as a camelCase string.</summary>
public enum ActivityReason
{
    Shared,
    Unshared,
    Attached,
    Detached,
    Plugged,
    Unplugged,
    ClientAdded,
    ClientRevoked,
    SecretRotated,
    Reconfigured,
    LinkUp,
    LinkDown,
    SignInFailed,
    PasswordChanged,
    SigningKeyRotated,
    CertificatePrepared,
    CertificateCancelled,
    CertificateSwitched,
    ServerMoved,
}

/// <summary>Whether the thing the event describes happened.</summary>
public enum ActivityOutcome
{
    Succeeded,
    Failed,
}

/// <summary>
/// One thing that happened, as the server saw it. <see cref="Summary"/> is the sentence the UI shows;
/// <see cref="Detail"/> carries the reason when <see cref="Outcome"/> is Failed.
/// </summary>
/// <remarks>Same shape as Timebase.Core.Activity's ActivityEvent, plus a ready-made summary.</remarks>
public sealed record ActivityEvent(
    string Id,
    DateTimeOffset TimestampUtc,
    EntityRef Entity,
    ActivityReason Reason,
    ActivityOutcome Outcome,
    string Summary,
    string? Detail = null);

/// <summary>Records activity. The store stamps the id and time.</summary>
public interface IActivityRecorder
{
    Task<ActivityEvent> RecordAsync(EntityRef entity, ActivityReason reason, ActivityOutcome outcome, string summary, string? detail = null);
}

/// <summary>Reads recent activity, newest first.</summary>
public interface IActivityReader
{
    IReadOnlyList<ActivityEvent> Recent(int max, string? entityId = null);
}

/// <summary>
/// A bounded in-memory log, like Timebase's InMemoryActivityStore: it survives page reloads and is
/// shared by every browser, but starts empty when the process restarts.
/// </summary>
public sealed class InMemoryActivityStore(TimeProvider time, int capacity = 500, ILogger<InMemoryActivityStore>? logger = null) : IActivityRecorder, IActivityReader
{
    private readonly LinkedList<ActivityEvent> _events = new();
    private readonly Lock _gate = new();

    /// <summary>Raised after each event is stored, for pushing it to live UIs.</summary>
    public event Func<ActivityEvent, Task>? Recorded;

    public async Task<ActivityEvent> RecordAsync(EntityRef entity, ActivityReason reason, ActivityOutcome outcome, string summary, string? detail = null)
    {
        var activity = new ActivityEvent(Guid.CreateVersion7().ToString("N"), time.GetUtcNow(), entity, reason, outcome, summary, detail);
        lock (_gate)
        {
            _events.AddFirst(activity);
            while (_events.Count > capacity)
            {
                _events.RemoveLast();
            }
        }

        // A handler that fails must not fail the caller that only recorded something.
        await Recorded.RaiseAsync(activity, logger ?? NullLogger<InMemoryActivityStore>.Instance);

        return activity;
    }

    public IReadOnlyList<ActivityEvent> Recent(int max, string? entityId = null)
    {
        lock (_gate)
        {
            return [.. _events.Where(e => entityId is null || e.Entity.Id == entityId).Take(max)];
        }
    }
}

public static class ActivityEndpoints
{
    /// <summary><c>GET /api/activity?entity=&amp;max=</c>: recent events, newest first.</summary>
    public static IEndpointRouteBuilder MapActivityEndpoints(this IEndpointRouteBuilder app)
    {
        app.MapGet("/api/activity", (IActivityReader reader, string? entity, int? max) =>
            TypedResults.Ok(reader.Recent(Math.Clamp(max ?? 100, 1, 500), entity)));
        return app;
    }
}
