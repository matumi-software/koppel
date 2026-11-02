using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Koppel.Settings;
using Microsoft.AspNetCore.SignalR.Client;

namespace Koppel.Tests.Settings;

/// <summary>
/// The product rule end to end: PATCH answers 202 with no body, and the confirmed settings
/// arrive over SignalR only after they are on disk.
/// </summary>
public sealed class SettingsConfirmFlowTests : IDisposable
{
    private readonly KoppelFactory _koppel = new();

    public void Dispose() => _koppel.Dispose();

    [Fact]
    public async Task Patch_is_confirmed_over_SignalR_after_the_file_is_written()
    {
        var ct = TestContext.Current.CancellationToken;
        using var client = _koppel.CreateUiClient();
        await using var hub = await _koppel.ConnectUiHubAsync(ct);
        var confirmed = new TaskCompletionSource<JsonElement>(TaskCreationOptions.RunContinuationsAsynchronously);
        hub.On<JsonElement>("SettingsChanged", settings => confirmed.TrySetResult(settings));

        using var response = await client.PatchAsJsonAsync("/api/settings", new { shareOnStartup = false }, ct);

        response.StatusCode.Should().Be(HttpStatusCode.Accepted);
        (await response.Content.ReadAsStringAsync(ct)).Should().BeEmpty();
        var pushed = await confirmed.Task.WaitAsync(TimeSpan.FromSeconds(5), ct);
        pushed.GetProperty("shareOnStartup").GetBoolean().Should().BeFalse();
        var onDisk = JsonSerializer.Deserialize<KoppelSettings>(await File.ReadAllTextAsync(_koppel.ConfigFile("settings.json"), ct), KoppelJson.Files)!;
        onDisk.ShareOnStartup.Should().BeFalse();
    }

    [Fact]
    public async Task Patch_with_a_bad_port_is_rejected_and_nothing_is_pushed()
    {
        var ct = TestContext.Current.CancellationToken;
        using var client = _koppel.CreateUiClient();
        await using var hub = await _koppel.ConnectUiHubAsync(ct);
        var pushes = new System.Collections.Concurrent.ConcurrentQueue<JsonElement>();
        hub.On<JsonElement>("SettingsChanged", pushes.Enqueue);

        using var rejected = await client.PatchAsJsonAsync("/api/settings", new { listenPort = 70000 }, ct);

        rejected.StatusCode.Should().Be(HttpStatusCode.BadRequest);

        // A valid patch afterwards is the sentinel: pushes arrive in order, so once it is
        // here a push from the rejected patch would be too, with no sleep to guess at.
        using var sentinel = await client.PatchAsJsonAsync("/api/settings", new { shareOnStartup = false }, ct);
        await WaitForAsync(() => !pushes.IsEmpty, ct);

        pushes.Should().ContainSingle().Which.GetProperty("listenPort").GetInt32().Should().Be(_koppel.UsbPort);
        (await client.GetFromJsonAsync<JsonElement>("/api/settings", ct)).GetProperty("listenPort").GetInt32().Should().Be(_koppel.UsbPort);
    }

    private static async Task WaitForAsync(Func<bool> condition, CancellationToken cancellationToken)
    {
        var deadline = DateTime.UtcNow.AddSeconds(5);
        while (!condition())
        {
            if (DateTime.UtcNow > deadline)
            {
                throw new TimeoutException("The push did not arrive.");
            }

            await Task.Delay(20, cancellationToken);
        }
    }
}
