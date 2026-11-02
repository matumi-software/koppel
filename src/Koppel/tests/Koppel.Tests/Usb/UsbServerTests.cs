using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Koppel.Core.Usb;
using Microsoft.Extensions.DependencyInjection;

namespace Koppel.Tests.Usb;

/// <summary>What the server decides to share: the saved choice, the device on the port, and what the kernel took.</summary>
public sealed class UsbServerTests : IDisposable
{
    private readonly FakeKernel _kernel = new();
    private KoppelFactory? _host;

    public void Dispose() => _host?.Dispose();

    [Fact]
    public async Task A_different_device_on_a_shared_port_is_unbound_and_not_exportable()
    {
        var ct = TestContext.Current.CancellationToken;
        _kernel.Plug(FakeKernel.Dongle with { VendorId = 0x046d, ProductId = 0xc548, Product = "Other device" });
        _host = new KoppelFactory(services: s => s.AddSingleton<IUsbHost>(_kernel), shareDongle: true);
        using var ui = _host.CreateUiClient();

        var devices = await ui.GetFromJsonAsync<JsonElement>("/api/server/devices", ct);

        _kernel.Unshared.Should().Contain("1-1.4");
        var device = devices.GetProperty("devices").EnumerateArray().Single();
        device.GetProperty("state").GetString().Should().Be("local");
        device.GetProperty("fault").GetString().Should().Contain("different device");
        var exportable = await _host.Services.GetRequiredService<Koppel.Usb.UsbServer>().ExportableAsync(ct);
        exportable.Should().BeEmpty();
    }

    [Fact]
    public async Task A_device_bound_on_the_port_but_not_the_shared_one_is_not_exportable_even_before_it_is_unbound()
    {
        var ct = TestContext.Current.CancellationToken;
        _host = new KoppelFactory(services: s => s.AddSingleton<IUsbHost>(_kernel), shareDongle: true);
        using var ui = _host.CreateUiClient();
        await ui.GetAsync("/api/server/devices", ct);

        _kernel.Plug(FakeKernel.Dongle with { VendorId = 0x046d, ProductId = 0xc548 });
        _kernel.Bind("1-1.4", true);

        var exportable = await _host.Services.GetRequiredService<Koppel.Usb.UsbServer>().ExportableAsync(ct);
        exportable.Should().BeEmpty();
    }

    [Theory]
    [InlineData("1-1%0A")]
    [InlineData("abc")]
    [InlineData("%DB%B1-1")]
    [InlineData("1-1.4.5.6.7.8.9.10.11.12.13.14.15.16.17.18.19.20")]
    public async Task A_bad_bus_id_is_refused_with_400_and_not_saved(string busId)
    {
        var ct = TestContext.Current.CancellationToken;
        _host = new KoppelFactory(services: s => s.AddSingleton<IUsbHost>(_kernel));
        using var ui = _host.CreateUiClient();

        using var response = await ui.PutAsJsonAsync($"/api/server/devices/{busId}/shared", new { shared = true }, ct);

        response.StatusCode.Should().Be(HttpStatusCode.BadRequest);
        _kernel.Shared.Should().BeEmpty();
        File.Exists(_host.ConfigFile("devices.json")).Should().BeTrue();
        File.ReadAllText(_host.ConfigFile("devices.json")).Should().NotContain("abc").And.NotContain("1-1\\n");
    }

    [Fact]
    public async Task A_share_the_kernel_refused_is_not_saved_and_shows_as_a_fault()
    {
        var ct = TestContext.Current.CancellationToken;
        _kernel.Bind("1-1.4", false);
        _kernel.ShareFailure = "The kernel refused it.";
        _host = new KoppelFactory(services: s => s.AddSingleton<IUsbHost>(_kernel));
        using var ui = _host.CreateUiClient();

        using var response = await ui.PutAsJsonAsync("/api/server/devices/1-1.4/shared", new { shared = true }, ct);

        response.StatusCode.Should().Be(HttpStatusCode.Accepted);
        File.ReadAllText(_host.ConfigFile("devices.json")).Should().NotContain("1-1.4");
        var devices = await ui.GetFromJsonAsync<JsonElement>("/api/server/devices", ct);
        devices.GetProperty("devices").EnumerateArray().Single().GetProperty("fault").GetString().Should().Be("The kernel refused it.");

        // Once the kernel agrees, the share is saved.
        _kernel.ShareFailure = null;
        (await ui.PutAsJsonAsync("/api/server/devices/1-1.4/shared", new { shared = true }, ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);
        File.ReadAllText(_host.ConfigFile("devices.json")).Should().Contain("1-1.4");
        _kernel.IsBound("1-1.4").Should().BeTrue();
    }

    [Fact]
    public async Task Start_up_unbinds_devices_bound_to_usbip_host_that_were_never_shared()
    {
        var ct = TestContext.Current.CancellationToken;
        _host = new KoppelFactory(services: s => s.AddSingleton<IUsbHost>(_kernel));
        using var ui = _host.CreateUiClient();

        var devices = await ui.GetFromJsonAsync<JsonElement>("/api/server/devices", ct);

        _kernel.IsBound("1-1.4").Should().BeFalse();
        devices.GetProperty("devices").EnumerateArray().Single().GetProperty("state").GetString().Should().Be("local");
    }

    [Fact]
    public async Task Start_up_unbinds_saved_shares_when_ShareOnStartup_is_off()
    {
        var ct = TestContext.Current.CancellationToken;
        _host = new KoppelFactory(services: s => s.AddSingleton<IUsbHost>(_kernel), shareDongle: true, shareOnStartup: false);
        using var ui = _host.CreateUiClient();

        var devices = await ui.GetFromJsonAsync<JsonElement>("/api/server/devices", ct);

        _kernel.IsBound("1-1.4").Should().BeFalse();
        devices.GetProperty("devices").EnumerateArray().Single().GetProperty("state").GetString().Should().Be("local");
        File.ReadAllText(_host.ConfigFile("devices.json")).Should().Contain("1-1.4", "the choice is kept for when the user shares it again");
    }

    [Fact]
    public async Task Start_up_keeps_what_was_shared_and_saved()
    {
        var ct = TestContext.Current.CancellationToken;
        _host = new KoppelFactory(services: s => s.AddSingleton<IUsbHost>(_kernel), shareDongle: true);
        using var ui = _host.CreateUiClient();

        var devices = await ui.GetFromJsonAsync<JsonElement>("/api/server/devices", ct);

        _kernel.IsBound("1-1.4").Should().BeTrue();
        _kernel.Unshared.Should().BeEmpty();
        devices.GetProperty("devices").EnumerateArray().Single().GetProperty("state").GetString().Should().Be("shared");
    }
}
