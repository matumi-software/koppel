using System.Collections.Concurrent;

namespace Koppel.Login;

/// <summary>
/// Slows down guessing. After <see cref="FreeFailures"/> failures a key is locked for a minute, and
/// every further failure doubles that, up to an hour. A success clears the key. The lock applies
/// before the secret is looked at, so a correct guess during it is refused too.
/// </summary>
internal sealed class UiLoginThrottle(TimeProvider time)
{
    public const int FreeFailures = 5;

    private static readonly TimeSpan FirstLock = TimeSpan.FromMinutes(1);
    private static readonly TimeSpan MaxLock = TimeSpan.FromHours(1);
    private static readonly TimeSpan Forget = TimeSpan.FromHours(2);
    private const int PruneAbove = 1000;

    private readonly ConcurrentDictionary<string, Entry> _entries = new();

    private sealed record Entry(int Failures, DateTimeOffset LockedUntil, DateTimeOffset Last);

    /// <summary>How long the key stays locked, or null when an attempt is allowed.</summary>
    public TimeSpan? LockedFor(string key)
    {
        if (_entries.TryGetValue(key, out var entry) && entry.LockedUntil - time.GetUtcNow() is { } left && left > TimeSpan.Zero)
        {
            return left;
        }

        return null;
    }

    public void Fail(string key)
    {
        var now = time.GetUtcNow();
        _entries.AddOrUpdate(
            key,
            _ => Next(new Entry(0, DateTimeOffset.MinValue, now), now),
            (_, entry) => Next(entry, now));
        if (_entries.Count > PruneAbove)
        {
            foreach (var (stale, _) in _entries.Where(e => now - e.Value.Last > Forget && e.Value.LockedUntil < now))
            {
                _entries.TryRemove(stale, out _);
            }
        }
    }

    public void Clear(string key) => _entries.TryRemove(key, out _);

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
