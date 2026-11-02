using Koppel.Core.Usb.Windows;

namespace Koppel.Core.Usb.Tests;

public sealed class UsbipWin2DriverTests : IDisposable
{
    private readonly string _directory = Path.Combine(Path.GetTempPath(), "koppel-tests", Guid.NewGuid().ToString("N"));

    public void Dispose()
    {
        if (Directory.Exists(_directory))
        {
            Directory.Delete(_directory, recursive: true);
        }
    }

    [Fact]
    public async Task Attach_goes_on_when_there_is_nothing_to_stop()
    {
        Assert.SkipUnless(OperatingSystem.IsWindows(), "Runs a batch file in place of usbip.exe.");
        var driver = new UsbipWin2Driver(FakeUsbip("""
            echo %* | findstr /C:"--stop" >nul && (echo error: no attach attempt to stop 1>&2 & exit /b 1)
            echo 7
            """));

        var port = await driver.AttachAsync("127.0.0.1", 3242, "1-1.4", TestContext.Current.CancellationToken);

        port.Should().Be(7);
    }

    [Fact]
    public async Task Detach_goes_on_when_there_is_nothing_to_stop()
    {
        Assert.SkipUnless(OperatingSystem.IsWindows(), "Runs a batch file in place of usbip.exe.");
        var log = Path.Combine(_directory, "calls.txt");
        var driver = new UsbipWin2Driver(FakeUsbip($"""
            echo %* >> "{log}"
            echo %* | findstr /C:"--stop" >nul && (echo error: no attach attempt to stop 1>&2 & exit /b 1)
            exit /b 0
            """));

        await driver.DetachAsync("127.0.0.1", 3242, "1-1.4", 7, TestContext.Current.CancellationToken);

        File.ReadAllText(log).Should().Contain("detach -p 7");
    }

    [Fact]
    public async Task Attach_still_fails_when_the_attach_itself_fails()
    {
        Assert.SkipUnless(OperatingSystem.IsWindows(), "Runs a batch file in place of usbip.exe.");
        var driver = new UsbipWin2Driver(FakeUsbip("""
            echo error: device busy 1>&2
            exit /b 1
            """));

        var attach = () => driver.AttachAsync("127.0.0.1", 3242, "1-1.4", TestContext.Current.CancellationToken);

        (await attach.Should().ThrowAsync<UsbHostException>()).Which.Message.Should().Be("device busy");
    }

    private string FakeUsbip(string body)
    {
        Directory.CreateDirectory(_directory);
        var path = Path.Combine(_directory, "usbip.cmd");
        File.WriteAllText(path, "@echo off\r\n" + body.ReplaceLineEndings("\r\n") + "\r\n");
        return path;
    }
}
