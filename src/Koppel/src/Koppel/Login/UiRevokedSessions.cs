namespace Koppel.Login;

/// <summary>
/// Session ids that signed out, kept until the session could no longer have been valid anyway, so a
/// copied cookie is refused. Bounded: past <see cref="MaxEntries"/> the ones that end soonest go.
/// </summary>
internal sealed class UiRevokedSessions(TimeProvider time)
{
    public const int MaxEntries = 4096;

    private readonly Lock _gate = new();
    private readonly Dictionary<string, DateTimeOffset> _revoked = [];

    public void Revoke(string? sessionId, DateTimeOffset until)
    {
        if (string.IsNullOrEmpty(sessionId))
        {
            return;
        }

        var now = time.GetUtcNow();
        lock (_gate)
        {
            if (_revoked.Count >= MaxEntries)
            {
                foreach (var gone in _revoked.Where(e => e.Value <= now).Select(e => e.Key).ToList())
                {
                    _revoked.Remove(gone);
                }

                while (_revoked.Count >= MaxEntries)
                {
                    _revoked.Remove(_revoked.MinBy(e => e.Value).Key);
                }
            }

            _revoked[sessionId] = until;
        }
    }

    public bool IsRevoked(string? sessionId)
    {
        if (sessionId is null)
        {
            return false;
        }

        lock (_gate)
        {
            return _revoked.TryGetValue(sessionId, out var until) && until > time.GetUtcNow();
        }
    }
}
