namespace Koppel.Login;

/// <summary>What <see cref="UiLoginThrottle.Reserve"/> decided.</summary>
/// <param name="Granted">False while the key is locked: the attempt must not be looked at.</param>
/// <param name="Lock">When refused, how long until another try. When granted, the lock this attempt engaged, or zero.</param>
/// <param name="Failures">With a grant: how many attempts count against the key now, this one included.</param>
internal readonly record struct Reservation(bool Granted, TimeSpan Lock, int Failures);

/// <summary>
/// Slows down guessing. An attempt is reserved before the secret is looked at and counts as a failure
/// until it succeeds, so parallel guesses cannot all slip in before the first one is booked. After
/// <see cref="FreeFailures"/> reserved attempts a key is locked for a minute, and every further one
/// doubles that, up to an hour. A success clears the key. The lock applies before the secret is looked
/// at, so a correct guess during it is refused too. The table is capped: when it is full the stale keys
/// go first, then the ones not seen for longest.
/// </summary>
internal sealed class UiLoginThrottle(TimeProvider time)
{
    public const int FreeFailures = 5;
    public const int MaxKeys = 2048;

    private static readonly TimeSpan FirstLock = TimeSpan.FromMinutes(1);
    private static readonly TimeSpan MaxLock = TimeSpan.FromHours(1);
    private static readonly TimeSpan Forget = TimeSpan.FromHours(2);

    private readonly Lock _gate = new();
    private readonly Dictionary<string, Entry> _entries = [];

    private sealed record Entry(int Failures, DateTimeOffset LockedUntil, DateTimeOffset Last);

    public int Count
    {
        get
        {
            lock (_gate)
            {
                return _entries.Count;
            }
        }
    }

    /// <summary>How long the key stays locked, or null when an attempt is allowed.</summary>
    public TimeSpan? LockedFor(string key)
    {
        lock (_gate)
        {
            return _entries.TryGetValue(key, out var entry) ? Remaining(entry, time.GetUtcNow()) : null;
        }
    }

    /// <summary>Books an attempt as a failure, atomically, unless the key is locked. Call <see cref="Clear"/> when it turns out right.</summary>
    public Reservation Reserve(string key)
    {
        var now = time.GetUtcNow();
        lock (_gate)
        {
            if (_entries.TryGetValue(key, out var entry))
            {
                if (Remaining(entry, now) is { } wait)
                {
                    return new Reservation(false, wait, entry.Failures);
                }
            }
            else
            {
                MakeRoom(now);
                entry = new Entry(0, DateTimeOffset.MinValue, now);
            }

            var next = Next(entry, now);
            _entries[key] = next;
            return new Reservation(true, Remaining(next, now) ?? TimeSpan.Zero, next.Failures);
        }
    }

    public void Clear(string key)
    {
        lock (_gate)
        {
            _entries.Remove(key);
        }
    }

    private static TimeSpan? Remaining(Entry entry, DateTimeOffset now) => entry.LockedUntil - now is { } left && left > TimeSpan.Zero ? left : null;

    private void MakeRoom(DateTimeOffset now)
    {
        if (_entries.Count < MaxKeys)
        {
            return;
        }

        foreach (var stale in _entries.Where(e => now - e.Value.Last > Forget && e.Value.LockedUntil < now).Select(e => e.Key).ToList())
        {
            _entries.Remove(stale);
        }

        // Still full: the key unseen for longest goes, so a flood of new callers cannot grow the table.
        while (_entries.Count >= MaxKeys)
        {
            _entries.Remove(_entries.MinBy(e => e.Value.Last).Key);
        }
    }

    private static Entry Next(Entry entry, DateTimeOffset now)
    {
        var failures = (now - entry.Last > Forget ? 0 : entry.Failures) + 1;
        if (failures < FreeFailures)
        {
            return new Entry(failures, DateTimeOffset.MinValue, now);
        }

        var seconds = Math.Min(FirstLock.TotalSeconds * Math.Pow(2, Math.Min(failures - FreeFailures, 20)), MaxLock.TotalSeconds);
        return new Entry(failures, now + TimeSpan.FromSeconds(seconds), now);
    }
}
