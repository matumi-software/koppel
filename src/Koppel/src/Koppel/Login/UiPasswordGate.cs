namespace Koppel.Login;

/// <summary>
/// How many password hashes run at once, for the whole server. Each one costs about 600,000 PBKDF2
/// rounds, so an anonymous caller must not be able to line up as many as it likes. Setup, login and
/// password change all count. When it is full the caller is told at once, nothing waits.
/// </summary>
internal sealed class UiPasswordGate : IDisposable
{
    public const int Slots = 2;

    private readonly SemaphoreSlim _slots = new(Slots, Slots);

    /// <summary>A slot, to dispose when the hash is done, or null when all are taken.</summary>
    public IDisposable? TryEnter() => _slots.Wait(0) ? new Lease(_slots) : null;

    public void Dispose() => _slots.Dispose();

    private sealed class Lease(SemaphoreSlim slots) : IDisposable
    {
        private int _released;

        public void Dispose()
        {
            if (Interlocked.Exchange(ref _released, 1) == 0)
            {
                slots.Release();
            }
        }
    }
}
