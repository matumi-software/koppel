using Koppel.Core.Activity;

namespace Koppel.Tests.Activity;

public sealed class InMemoryActivityStoreTests
{
    [Fact]
    public async Task A_handler_that_throws_does_not_fail_the_caller_or_the_handlers_after_it()
    {
        var store = new InMemoryActivityStore(TimeProvider.System);
        var heard = new TaskCompletionSource<ActivityEvent>();
        store.Recorded += _ => throw new InvalidOperationException("a UI that went away");
        store.Recorded += e =>
        {
            heard.SetResult(e);
            return Task.CompletedTask;
        };

        var recorded = await store.RecordAsync(new EntityRef("1-1", "Dongle", "device"), ActivityReason.Shared, ActivityOutcome.Succeeded, "Dongle is shared");

        (await heard.Task).Should().Be(recorded);
        store.Recent(10).Should().ContainSingle();
    }
}
