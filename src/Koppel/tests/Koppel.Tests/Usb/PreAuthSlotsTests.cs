using System.Net;
using Koppel.Usb;

namespace Koppel.Tests.Usb;

/// <summary>Who may wait on the USB port before authenticating, and who gets pushed out when it is full.</summary>
public sealed class PreAuthSlotsTests
{
    private readonly ManualTimeProvider _time = new();

    private static IPAddress V4(int last) => IPAddress.Parse($"192.0.2.{last}");

    [Fact]
    public void IPv6_addresses_in_one_64_share_a_bucket_and_IPv4_stands_for_itself()
    {
        PreAuthSlots.KeyOf(IPAddress.Parse("2001:db8:1:2::1")).Should().Be(PreAuthSlots.KeyOf(IPAddress.Parse("2001:db8:1:2:ffff:ffff:ffff:ffff")));
        PreAuthSlots.KeyOf(IPAddress.Parse("2001:db8:1:2::1")).Should().NotBe(PreAuthSlots.KeyOf(IPAddress.Parse("2001:db8:1:3::1")));
        PreAuthSlots.KeyOf(V4(7)).Should().Be(V4(7));
    }

    [Fact]
    public void A_host_with_many_IPv6_addresses_cannot_hold_more_than_its_share()
    {
        var slots = new PreAuthSlots(_time);
        var held = Enumerable.Range(1, 20).Select(i => slots.Enter(IPAddress.Parse($"2001:db8:1:2::{i:x}"))).ToList();

        held.Count(s => !s.Evicted.IsCancellationRequested).Should().Be(8);
        held.Take(12).Should().OnlyContain(s => s.Evicted.IsCancellationRequested, "the oldest are the ones dropped");
    }

    [Fact]
    public void When_the_pool_is_full_the_oldest_waiting_connection_is_dropped_and_the_newest_gets_in()
    {
        var slots = new PreAuthSlots(_time, capacity: 10, reserve: 2, perAddress: 100);
        var held = Enumerable.Range(1, 8).Select(i => slots.Enter(V4(i))).ToList();

        var newest = slots.Enter(V4(100));

        held[0].Evicted.IsCancellationRequested.Should().BeTrue();
        held.Skip(1).Should().OnlyContain(s => !s.Evicted.IsCancellationRequested);
        newest.Evicted.IsCancellationRequested.Should().BeFalse();
        slots.Count.Should().Be(8);
    }

    [Fact]
    public void A_flood_from_elsewhere_cannot_push_out_an_address_that_authenticated_recently()
    {
        var slots = new PreAuthSlots(_time, capacity: 10, reserve: 2, perAddress: 100);
        slots.Enter(V4(1)).Authenticated();
        var regular = slots.Enter(V4(1));

        var flood = Enumerable.Range(10, 100).Select(i => slots.Enter(V4(i))).ToList();

        regular.Recent.Should().BeTrue();
        regular.Evicted.IsCancellationRequested.Should().BeFalse("the reserve is for addresses that were seen authenticating");
        flood.Count(s => !s.Evicted.IsCancellationRequested).Should().Be(8);
    }

    [Fact]
    public void An_address_is_recent_for_ten_minutes()
    {
        var slots = new PreAuthSlots(_time);
        slots.Enter(V4(1)).Authenticated();

        slots.Enter(V4(1)).Recent.Should().BeTrue();
        _time.Advance(TimeSpan.FromMinutes(11));

        slots.Enter(V4(1)).Recent.Should().BeFalse();
    }

    [Fact]
    public void A_slot_that_authenticated_or_left_stops_counting()
    {
        var slots = new PreAuthSlots(_time);
        var done = slots.Enter(V4(1));
        var gone = slots.Enter(V4(2));

        done.Authenticated();
        gone.Dispose();

        slots.Count.Should().Be(0);
        done.Evicted.IsCancellationRequested.Should().BeFalse();
    }
}
