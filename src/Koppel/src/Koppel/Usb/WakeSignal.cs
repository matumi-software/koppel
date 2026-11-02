using System.Threading.Channels;

namespace Koppel.Usb;

/// <summary>
/// A wake-up that cannot pile up: any number of <see cref="Set"/> calls before a wait count as one, so a
/// burst of changes costs one pass of the loop that waits, not one per change.
/// </summary>
internal sealed class WakeSignal
{
    private readonly Channel<bool> _channel = Channel.CreateBounded<bool>(
        new BoundedChannelOptions(1) { FullMode = BoundedChannelFullMode.DropWrite, SingleReader = true });

    public void Set() => _channel.Writer.TryWrite(true);

    /// <summary>Forgets a wake-up nobody has waited for yet.</summary>
    public void Reset() => _channel.Reader.TryRead(out _);

    public async Task WaitAsync(CancellationToken cancellationToken) => await _channel.Reader.ReadAsync(cancellationToken);

    /// <summary>Waits for <see cref="Set"/> or for <paramref name="timeout"/>, whichever comes first.</summary>
    public async Task WaitAsync(TimeSpan timeout, CancellationToken cancellationToken)
    {
        using var limit = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        limit.CancelAfter(timeout);
        try
        {
            await _channel.Reader.ReadAsync(limit.Token);
        }
        catch (OperationCanceledException) when (!cancellationToken.IsCancellationRequested)
        {
            // The time ran out.
        }
    }
}
