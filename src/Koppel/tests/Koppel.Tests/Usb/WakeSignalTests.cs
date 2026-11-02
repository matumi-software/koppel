using Koppel.Usb;

namespace Koppel.Tests.Usb;

public sealed class WakeSignalTests
{
    [Fact]
    public async Task Many_sets_before_a_wait_wake_it_once()
    {
        var ct = TestContext.Current.CancellationToken;
        var signal = new WakeSignal();
        for (var i = 0; i < 50; i++)
        {
            signal.Set();
        }

        await signal.WaitAsync(ct).WaitAsync(TimeSpan.FromSeconds(5), ct);

        // Nothing is left queued: the next wait is pending, where a counting semaphore would pass again.
        var second = signal.WaitAsync(CancellationToken.None);
        second.IsCompleted.Should().BeFalse();
        signal.Set();
        await second.WaitAsync(TimeSpan.FromSeconds(5), ct);
    }

    [Fact]
    public async Task Reset_forgets_a_wake_up_nobody_waited_for()
    {
        var ct = TestContext.Current.CancellationToken;
        var signal = new WakeSignal();
        signal.Set();
        signal.Reset();

        var wait = signal.WaitAsync(CancellationToken.None);

        wait.IsCompleted.Should().BeFalse();
        signal.Set();
        await wait.WaitAsync(TimeSpan.FromSeconds(5), ct);
    }

    [Fact]
    public async Task A_wait_with_a_time_limit_returns_when_set()
    {
        var ct = TestContext.Current.CancellationToken;
        var signal = new WakeSignal();
        var wait = signal.WaitAsync(TimeSpan.FromMinutes(5), ct);

        signal.Set();

        await wait.WaitAsync(TimeSpan.FromSeconds(5), ct);
    }
}
