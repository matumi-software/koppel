using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Koppel.Core.Usb;
using Microsoft.Extensions.DependencyInjection;

namespace Koppel.Tests.Usb;

/// <summary>
/// <c>devices.json</c> and <c>attached.json</c> are plain, non-critical files: a corrupt one must not stop
/// Koppel from starting, and must not be overwritten until the user changes something.
/// </summary>
public sealed class PlainFileStartupTests : IDisposable
{
    private const string Corrupt = "{ not json";

    private readonly FakeKernel _kernel = new();
    private readonly KoppelFactory _host;

    public PlainFileStartupTests()
    {
        _kernel.Plug(FakeKernel.Dongle);
        _host = new KoppelFactory(services: s =>
        {
            s.AddSingleton<IUsbHost>(_kernel);
            s.AddSingleton<IUsbAttacher>(_kernel);
        });
    }

    public void Dispose() => _host.Dispose();

    [Theory]
    [InlineData("devices.json", "devices")]
    [InlineData("attached.json", "attached")]
    public async Task A_corrupt_plain_file_does_not_stop_start_and_shows_as_a_system_check(string file, string checkId)
    {
        var ct = TestContext.Current.CancellationToken;
        await File.WriteAllTextAsync(_host.ConfigFile(file), Corrupt, ct);
        using var ui = _host.CreateUiClient();

        var checks = await ui.GetFromJsonAsync<JsonElement>("/api/system/checks", ct);

        var check = checks.EnumerateArray().Single(c => c.GetProperty("id").GetString() == checkId);
        check.GetProperty("ok").GetBoolean().Should().BeFalse();
        check.GetProperty("title").GetString().Should().Contain(file);
        check.GetProperty("detail").GetString().Should().Contain("not valid JSON");
        (await File.ReadAllTextAsync(_host.ConfigFile(file), ct)).Should().Be(Corrupt, "the file is left as it was until the user changes something");
    }

    [Fact]
    public async Task A_healthy_host_has_no_plain_file_checks()
    {
        var ct = TestContext.Current.CancellationToken;
        using var ui = _host.CreateUiClient();

        var checks = await ui.GetFromJsonAsync<JsonElement>("/api/system/checks", ct);

        checks.EnumerateArray().Select(c => c.GetProperty("id").GetString()).Should().NotContain(["devices", "attached"]);
    }

    [Fact]
    public async Task Sharing_a_device_over_a_corrupt_devices_file_moves_it_aside_and_writes_a_fresh_one()
    {
        var ct = TestContext.Current.CancellationToken;
        await File.WriteAllTextAsync(_host.ConfigFile("devices.json"), Corrupt, ct);
        using var ui = _host.CreateUiClient();

        (await ui.PutAsJsonAsync("/api/server/devices/1-1.4/shared", new { shared = true }, ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);

        (await File.ReadAllTextAsync(_host.ConfigFile("devices.json"), ct)).Should().Contain("1-1.4");
        var aside = Directory.GetFiles(Path.GetDirectoryName(_host.ConfigFile("devices.json"))!, "devices.json.unreadable-*").Should().ContainSingle().Subject;
        (await File.ReadAllTextAsync(aside, ct)).Should().Be(Corrupt);
        var checks = await ui.GetFromJsonAsync<JsonElement>("/api/system/checks", ct);
        checks.EnumerateArray().Select(c => c.GetProperty("id").GetString()).Should().NotContain("devices");
    }
}
