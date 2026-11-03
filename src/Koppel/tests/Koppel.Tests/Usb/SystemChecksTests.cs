using Koppel.Core.Usb;

namespace Koppel.Tests.Usb;

public sealed class SystemChecksTests
{
    private static readonly UnsupportedUsb Missing = new("The Windows USB driver is not installed (usbip.exe is missing). Reinstall Koppel to add it.");
    private static readonly FakeKernel Kernel = new();

    [Fact]
    public void Attach_on_native_windows_passes_when_the_driver_is_installed()
    {
        var check = SystemEndpoints.AttachCheck(Missing, new WorkingDriver(), nativeWindows: true);

        check.Ok.Should().BeTrue();
        check.Title.Should().Contain("usbip-win2");
    }

    [Fact]
    public void Attach_on_native_windows_fails_without_the_driver_and_says_to_reinstall()
    {
        var check = SystemEndpoints.AttachCheck(Kernel, Missing, nativeWindows: true);

        check.Ok.Should().BeFalse();
        check.Detail.Should().Contain("Reinstall Koppel");
    }

    [Fact]
    public void Attach_off_windows_still_reads_the_attacher()
    {
        SystemEndpoints.AttachCheck(Missing, new WorkingDriver(), nativeWindows: false).Ok.Should().BeFalse();
        SystemEndpoints.AttachCheck(Kernel, Missing, nativeWindows: false).Title.Should().Be("vhci-hcd is loaded.");
    }

    [Fact]
    public void Share_on_native_windows_without_an_upstream_names_usbipd_win_and_the_setting()
    {
        var check = SystemEndpoints.ShareCheck(Missing, nativeWindows: true);

        check.Ok.Should().BeFalse();
        check.Detail.Should().Contain("usbipd-win").And.Contain("Usb:Upstream");
    }

    [Fact]
    public void Share_off_windows_keeps_the_host_message()
    {
        SystemEndpoints.ShareCheck(Missing, nativeWindows: false).Detail.Should().Be(Missing.Unavailable);
        SystemEndpoints.ShareCheck(Kernel, nativeWindows: false).Title.Should().Be("usbip-host is loaded.");
    }

    private sealed class WorkingDriver : IUsbIpDriver
    {
        public string? Unavailable => null;

        public Task<int> AttachAsync(string host, int port, string busId, CancellationToken cancellationToken) => Task.FromResult(0);

        public Task DetachAsync(string host, int port, string busId, int driverPort, CancellationToken cancellationToken) => Task.CompletedTask;
    }
}
