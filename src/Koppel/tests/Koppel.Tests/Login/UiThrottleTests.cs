using System.Security.Claims;
using Koppel.Login;

namespace Koppel.Tests.Login;

public sealed class UiThrottleReservationTests
{
    [Fact]
    public void Parallel_reservations_on_one_key_grant_only_the_free_failures()
    {
        var throttle = new UiLoginThrottle(new ManualTimeProvider());
        var granted = 0;

        Parallel.For(0, 200, new ParallelOptions { MaxDegreeOfParallelism = 32 }, _ =>
        {
            if (throttle.Reserve("ip").Granted)
            {
                Interlocked.Increment(ref granted);
            }
        });

        granted.Should().Be(UiLoginThrottle.FreeFailures);
    }

    [Fact]
    public void The_attempt_that_engages_a_lock_says_so_and_later_ones_are_refused_with_the_wait()
    {
        var throttle = new UiLoginThrottle(new ManualTimeProvider());
        for (var i = 0; i < UiLoginThrottle.FreeFailures - 1; i++)
        {
            throttle.Reserve("ip").Lock.Should().Be(TimeSpan.Zero);
        }

        throttle.Reserve("ip").Lock.Should().Be(TimeSpan.FromMinutes(1));
        var refused = throttle.Reserve("ip");
        refused.Granted.Should().BeFalse();
        refused.Lock.Should().Be(TimeSpan.FromMinutes(1));
    }

    [Fact]
    public void A_cleared_key_starts_again()
    {
        var throttle = new UiLoginThrottle(new ManualTimeProvider());
        for (var i = 0; i < UiLoginThrottle.FreeFailures; i++)
        {
            throttle.Reserve("ip");
        }

        throttle.Clear("ip");

        throttle.Reserve("ip").Should().Be(new Reservation(true, TimeSpan.Zero, 1));
    }

    [Fact]
    public void The_table_never_grows_past_its_cap_and_drops_the_key_unseen_for_longest()
    {
        var clock = new ManualTimeProvider();
        var throttle = new UiLoginThrottle(clock);
        throttle.Reserve("first");
        for (var i = 1; i < UiLoginThrottle.MaxKeys + 500; i++)
        {
            clock.Advance(TimeSpan.FromMilliseconds(1));
            throttle.Reserve($"caller-{i}");
        }

        throttle.Count.Should().BeLessThanOrEqualTo(UiLoginThrottle.MaxKeys);
        throttle.Reserve("first").Failures.Should().Be(1, "its old failures were forgotten with it");
    }
}

public sealed class UiPasswordGateTests
{
    [Fact]
    public void Only_a_few_hashes_run_at_once_and_a_freed_slot_can_be_taken_again()
    {
        var gate = new UiPasswordGate();
        var held = Enumerable.Range(0, UiPasswordGate.Slots).Select(_ => gate.TryEnter()).ToList();

        held.Should().OnlyContain(slot => slot != null);
        gate.TryEnter().Should().BeNull("the caller is told at once, nothing waits");
        held[0]!.Dispose();
        held[0]!.Dispose();
        using var again = gate.TryEnter();
        again.Should().NotBeNull();
        gate.TryEnter().Should().BeNull("a slot is only returned once, however often its lease is disposed");
    }
}

public sealed class UiSessionTests
{
    [Fact]
    public void A_revoked_session_is_refused_until_it_would_have_ended_and_the_table_is_bounded()
    {
        var clock = new ManualTimeProvider();
        var revoked = new UiRevokedSessions(clock);
        revoked.Revoke("a", clock.GetUtcNow() + TimeSpan.FromHours(1));

        revoked.IsRevoked("a").Should().BeTrue();
        revoked.IsRevoked("b").Should().BeFalse();
        clock.Advance(TimeSpan.FromHours(2));
        revoked.IsRevoked("a").Should().BeFalse("it would have expired by now, so it need not be remembered");

        for (var i = 0; i < UiRevokedSessions.MaxEntries + 10; i++)
        {
            revoked.Revoke($"s{i}", clock.GetUtcNow() + TimeSpan.FromHours(1) + TimeSpan.FromSeconds(i));
        }

        revoked.IsRevoked($"s{UiRevokedSessions.MaxEntries + 9}").Should().BeTrue();
        revoked.IsRevoked("s0").Should().BeFalse("the one that ends soonest made room");
    }

    [Fact]
    public void A_session_ends_seven_days_after_it_was_first_signed_in_whatever_its_cookie_says()
    {
        var issued = DateTimeOffset.UnixEpoch + TimeSpan.FromDays(20000);

        var principal = UiSession.NewPrincipal("stamp", issued);

        UiSession.EndOf(principal).Should().Be(issued + TimeSpan.FromDays(7));
        UiSession.EndOf(new ClaimsPrincipal(new ClaimsIdentity())).Should().BeBefore(DateTimeOffset.UtcNow, "a cookie that does not say when it began is refused");
        var renewed = UiSession.NewPrincipal("newer", UiSession.IssuedOf(principal)!.Value, UiSession.SessionOf(principal));
        UiSession.EndOf(renewed).Should().Be(UiSession.EndOf(principal), "a password change does not restart the cap");
    }
}
