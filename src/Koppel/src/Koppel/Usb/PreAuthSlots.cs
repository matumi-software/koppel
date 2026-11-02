using System.Net;

namespace Koppel.Usb;

/// <summary>
/// The places a USB connection can wait in before it has proved who it is. The newest connection is always
/// let in; when its pool is full, the oldest one waiting there is dropped instead. An attacker who opens
/// many sockets can therefore only push out other waiting sockets, never keep a client from connecting.
/// </summary>
/// <remarks>
/// Callers are grouped by address, with IPv6 grouped by /64, because one host usually holds a whole /64
/// and can use any address in it. Part of the capacity is reserved for addresses that authenticated
/// recently, so a flood from elsewhere does not push out the clients that use this server.
/// </remarks>
internal sealed class PreAuthSlots(TimeProvider time, int capacity = 64, int reserve = 16, int perAddress = 8)
{
    private static readonly TimeSpan RecentFor = TimeSpan.FromMinutes(10);
    private const int MaxRecent = 1024;

    private readonly Lock _gate = new();
    private readonly LinkedList<Slot> _waiting = new();
    private readonly Dictionary<IPAddress, DateTimeOffset> _recent = new();

    public int Count
    {
        get
        {
            lock (_gate)
            {
                return _waiting.Count;
            }
        }
    }

    /// <summary>IPv4 addresses stand for themselves. An IPv6 address stands for its /64.</summary>
    public static IPAddress KeyOf(IPAddress address)
    {
        if (address.AddressFamily != System.Net.Sockets.AddressFamily.InterNetworkV6)
        {
            return address;
        }

        var bytes = address.GetAddressBytes();
        Array.Clear(bytes, 8, 8);
        return new IPAddress(bytes);
    }

    /// <summary>Takes a place for a new connection. The slot's token is cancelled if a newer connection pushes this one out.</summary>
    public Slot Enter(IPAddress address)
    {
        var key = KeyOf(address);
        Slot? first = null;
        Slot? second = null;
        Slot slot;
        lock (_gate)
        {
            var recent = IsRecent(key);
            slot = new Slot(this, key, recent);
            first = Evict(s => s.Key.Equals(key), perAddress);
            second = Evict(s => s.Recent == recent, recent ? reserve : capacity - reserve);
            slot.Node = _waiting.AddLast(slot);
        }

        first?.Cancel();
        second?.Cancel();
        return slot;
    }

    /// <summary>Removes the oldest slot that matches once <paramref name="limit"/> of them are waiting.</summary>
    private Slot? Evict(Func<Slot, bool> matches, int limit)
    {
        var count = _waiting.Count(matches);
        if (count < limit)
        {
            return null;
        }

        var victim = _waiting.First(matches);
        Remove(victim);
        return victim;
    }

    private bool IsRecent(IPAddress key) =>
        _recent.TryGetValue(key, out var at) && time.GetUtcNow() - at < RecentFor;

    private void Remove(Slot slot)
    {
        if (slot.Node is { } node)
        {
            slot.Node = null;
            _waiting.Remove(node);
        }
    }

    private void Authenticated(Slot slot)
    {
        lock (_gate)
        {
            Remove(slot);
            var now = time.GetUtcNow();
            if (_recent.Count >= MaxRecent)
            {
                foreach (var stale in _recent.Where(p => now - p.Value >= RecentFor).Select(p => p.Key).ToList())
                {
                    _recent.Remove(stale);
                }

                if (_recent.Count >= MaxRecent)
                {
                    _recent.Remove(_recent.MinBy(p => p.Value).Key);
                }
            }

            _recent[slot.Key] = now;
        }
    }

    private void Leave(Slot slot)
    {
        lock (_gate)
        {
            Remove(slot);
        }
    }

    /// <summary>One connection's place. Dispose it when the connection leaves the waiting state, one way or the other.</summary>
    internal sealed class Slot : IDisposable
    {
        private readonly PreAuthSlots _owner;
        private readonly CancellationTokenSource _evicted = new();

        internal Slot(PreAuthSlots owner, IPAddress key, bool recent)
        {
            _owner = owner;
            Key = key;
            Recent = recent;
        }

        internal LinkedListNode<Slot>? Node { get; set; }

        public IPAddress Key { get; }

        public bool Recent { get; }

        /// <summary>Cancelled when a newer connection took this one's place.</summary>
        public CancellationToken Evicted => _evicted.Token;

        /// <summary>The connection authenticated: it stops waiting, and its address counts as recently seen.</summary>
        public void Authenticated() => _owner.Authenticated(this);

        public void Cancel()
        {
            try
            {
                _evicted.Cancel();
            }
            catch (ObjectDisposedException)
            {
                // The connection ended by itself first.
            }
        }

        public void Dispose()
        {
            _owner.Leave(this);
            _evicted.Dispose();
        }
    }
}
