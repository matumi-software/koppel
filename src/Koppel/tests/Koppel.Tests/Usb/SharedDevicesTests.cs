using Koppel.Core.Usb;
using Koppel.Usb;

namespace Koppel.Tests.Usb;

public sealed class SharedDevicesTests
{
    [Fact]
    public void A_different_device_on_a_shared_port_is_not_shared_again()
    {
        var shared = new SharedDevices(["1-1"], new Dictionary<string, string> { ["1-1"] = "1209:0001" });

        shared.Expects(FakeKernel.Dongle with { BusId = "1-1", VendorId = 0x1209, ProductId = 0x0001 }).Should().BeTrue();
        shared.Expects(FakeKernel.Dongle with { BusId = "1-1", VendorId = 0x046d, ProductId = 0xc548 }).Should().BeFalse();
        shared.Expects(FakeKernel.Dongle with { BusId = "2-1" }).Should().BeFalse();
    }

    [Fact]
    public void Shares_saved_before_VID_PID_was_recorded_still_match_by_port()
    {
        var shared = new SharedDevices(["1-1"]);

        shared.Expects(FakeKernel.Dongle with { BusId = "1-1" }).Should().BeTrue();
    }
}
