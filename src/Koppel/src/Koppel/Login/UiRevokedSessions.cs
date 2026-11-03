namespace Koppel.Login;

/// <summary>
/// Session ids that signed out, kept in the encrypted <c>ui-auth.json</c> until the session could no
/// longer have been valid anyway, so a copied cookie is refused even after a restart. Bounded: an id
/// is dropped once its session has ended, and past <see cref="MaxEntries"/> the ones that end soonest go.
/// </summary>
internal sealed class UiRevokedSessions(TimeProvider time, UiAuthStore store)
{
    public const int MaxEntries = 4096;

    public Task RevokeAsync(string? sessionId, DateTimeOffset until, CancellationToken cancellationToken) =>
        string.IsNullOrEmpty(sessionId)
            ? Task.CompletedTask
            : store.UpdateRevokedSessionsAsync(current => With(current, sessionId, until, time.GetUtcNow()), cancellationToken);

    public bool IsRevoked(string? sessionId) => IsRevoked(store.RevokedSessions, sessionId, time.GetUtcNow());

    internal static bool IsRevoked(IReadOnlyDictionary<string, DateTimeOffset> revoked, string? sessionId, DateTimeOffset now) =>
        sessionId is not null && revoked.TryGetValue(sessionId, out var until) && until > now;

    /// <summary>The list with <paramref name="sessionId"/> added, the ended sessions dropped and room made when it is full.</summary>
    internal static Dictionary<string, DateTimeOffset> With(IReadOnlyDictionary<string, DateTimeOffset> current, string sessionId, DateTimeOffset until, DateTimeOffset now)
    {
        var next = current.Where(e => e.Value > now && e.Key != sessionId).ToDictionary();
        while (next.Count >= MaxEntries)
        {
            next.Remove(next.MinBy(e => e.Value).Key);
        }

        next[sessionId] = until;
        return next;
    }
}
