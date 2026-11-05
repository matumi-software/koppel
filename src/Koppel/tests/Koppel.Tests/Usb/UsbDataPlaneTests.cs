using System.Net;
using System.Net.Http.Headers;
using System.Net.Http.Json;
using Koppel.Core.Usb;
using Microsoft.Extensions.DependencyInjection;

namespace Koppel.Tests.Usb;

/// <summary>The USB data plane's WebSocket: who gets in, on which port, and what a role change does to live sessions.</summary>
public sealed class UsbDataPlaneTests : IDisposable
{
    private readonly FakeKernel _kernel = new();
    private readonly KoppelFactory _server;

    public UsbDataPlaneTests() => _server = new KoppelFactory(kestrel: true, services: s => s.AddSingleton<IUsbHost>(_kernel), shareDongle: true);

    public void Dispose() => _server.Dispose();

    [Fact]
    public async Task An_empty_token_is_denied()
    {
        var ct = TestContext.Current.CancellationToken;

        var (session, answer) = await RemoteAccess.OpenUsbAsync(_server, "", ct);
        await using var refused = session;

        answer.Should().Be("DENIED 401");
    }

    [Fact]
    public async Task A_plain_request_with_a_valid_token_is_not_a_session()
    {
        var ct = TestContext.Current.CancellationToken;
        var token = await TokenAsync(ct);
        using var https = RemoteAccess.Https(_server);
        using var request = new HttpRequestMessage(HttpMethod.Get, "/skakel/v1/streams/usb");
        request.Headers.Authorization = new AuthenticationHeaderValue("Bearer", token);

        using var response = await https.SendAsync(request, ct);

        response.StatusCode.Should().Be(HttpStatusCode.BadRequest);
    }

    [Fact]
    public async Task The_data_plane_is_not_served_on_the_ui_port()
    {
        var ct = TestContext.Current.CancellationToken;
        using var ui = _server.CreateUiClient();

        using var response = await ui.GetAsync("/skakel/v1/streams/usb", ct);

        response.StatusCode.Should().Be(HttpStatusCode.NotFound);
    }

    [Fact]
    public async Task Leaving_the_server_role_unshares_what_was_shared_and_closes_the_data_plane()
    {
        var ct = TestContext.Current.CancellationToken;
        var token = await TokenAsync(ct);
        _kernel.IsBound("1-1.4").Should().BeTrue();
        using var ui = _server.CreateUiClient();

        (await ui.PatchAsJsonAsync("/api/settings", new { role = "client" }, ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);

        await Eventually(() => Task.FromResult(!_kernel.IsBound("1-1.4")), ct);
        await Eventually(async () => await AnswerAsync(token, ct) == "DENIED 404", ct);

        (await ui.PatchAsJsonAsync("/api/settings", new { role = "server" }, ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);

        await Eventually(() => Task.FromResult(_kernel.IsBound("1-1.4")), ct);
        await Eventually(async () => await AnswerAsync(token, ct) == "OK", ct);
    }

    [Fact]
    public async Task Leaving_the_server_role_ends_live_sessions()
    {
        var ct = TestContext.Current.CancellationToken;
        var token = await TokenAsync(ct);
        var (session, answer) = await RemoteAccess.OpenUsbAsync(_server, token, ct);
        await using var live = session;
        answer.Should().Be("OK");
        await RemoteAccess.ImportAsync(session, "1-1.4", ct);

        using var ui = _server.CreateUiClient();
        (await ui.PatchAsJsonAsync("/api/settings", new { role = "client" }, ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);

        (await RemoteAccess.EndsWithinAsync(session, TimeSpan.FromSeconds(10), ct)).Should().BeTrue();
    }

    [Fact]
    public async Task A_handler_of_the_device_list_that_never_returns_does_not_stall_sharing_or_imports()
    {
        var ct = TestContext.Current.CancellationToken;
        var usb = _server.Services.GetRequiredService<Koppel.Usb.UsbServer>();
        var stuck = new TaskCompletionSource();
        usb.Changed += _ => stuck.Task;
        var (clientId, secret) = await RemoteAccess.CreateClientAsync(_server, "bench-3", ct);
        var token = await RemoteAccess.TokenAsync(_server, clientId, secret, ct);
        using var ui = _server.CreateUiClient();

        (await ui.PutAsJsonAsync("/api/server/devices/1-1.4/shared", new { shared = false }, ct).WaitAsync(TimeSpan.FromSeconds(10), ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);
        (await ui.PutAsJsonAsync("/api/server/devices/1-1.4/shared", new { shared = true }, ct).WaitAsync(TimeSpan.FromSeconds(10), ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);
        var (tls, answer) = await RemoteAccess.OpenUsbAsync(_server, token, ct);
        await using var session = tls;
        answer.Should().Be("OK");
        await RemoteAccess.ImportAsync(tls, "1-1.4", ct).WaitAsync(TimeSpan.FromSeconds(10), ct);

        await UsbEndToEndTests.EchoesAsync(tls, ct);
        stuck.SetResult();
    }

    [Fact]
    public async Task Snapshots_are_delivered_in_order_and_the_ones_a_newer_one_overtook_are_dropped()
    {
        var ct = TestContext.Current.CancellationToken;
        var usb = _server.Services.GetRequiredService<Koppel.Usb.UsbServer>();
        var first = new TaskCompletionSource();
        var release = new TaskCompletionSource();
        var delivered = new List<Koppel.Usb.ServerDevices>();
        usb.Changed += async devices =>
        {
            lock (delivered)
            {
                delivered.Add(devices);
            }

            first.TrySetResult();
            await release.Task;
        };
        using var ui = _server.CreateUiClient();
        await ui.PutAsJsonAsync("/api/server/devices/1-1.4/shared", new { shared = false }, ct);
        await first.Task.WaitAsync(TimeSpan.FromSeconds(10), ct);

        // The handler is busy with the first snapshot, so these queue up behind it.
        foreach (var shared in new[] { true, false, true })
        {
            await ui.PutAsJsonAsync("/api/server/devices/1-1.4/shared", new { shared }, ct);
        }

        var last = usb.Current;
        release.SetResult();
        await Eventually(() =>
        {
            lock (delivered)
            {
                return Task.FromResult(delivered.Count >= 2 && ReferenceEquals(delivered[^1], last));
            }
        }, ct);
        delivered.Count.Should().BeLessThanOrEqualTo(3, "the snapshots between the first and the newest were overtaken and dropped");
    }

    [Fact]
    public async Task Two_imports_of_one_device_do_not_reset_it_at_the_same_time()
    {
        var ct = TestContext.Current.CancellationToken;
        using var gate = new ManualResetEventSlim(false);
        _kernel.ResetGate = gate;
        var (clientId, secret) = await RemoteAccess.CreateClientAsync(_server, "bench-4", ct);
        var token = await RemoteAccess.TokenAsync(_server, clientId, secret, ct);
        var (firstTls, _) = await RemoteAccess.OpenUsbAsync(_server, token, ct);
        await using var firstSession = firstTls;
        var (secondTls, _) = await RemoteAccess.OpenUsbAsync(_server, token, ct);
        await using var secondSession = secondTls;
        try
        {
            await firstTls.WriteAsync(UsbIpProtocol.ImportRequest("1-1.4"), ct);
            await _kernel.ResetStarted.WaitAsync(TimeSpan.FromSeconds(10), ct);

            // The first import is inside its reset. The second must wait for it; without the lock it would reset too.
            // A second reset is the only thing that can happen here, so the wait is bounded by how long to look for it.
            await secondTls.WriteAsync(UsbIpProtocol.ImportRequest("1-1.4"), ct);
            (await _kernel.ResetStarted.WaitAsync(TimeSpan.FromSeconds(1), ct)).Should().BeFalse("the second import waits for the first one's reset");
        }
        finally
        {
            gate.Set();
        }

        (await UsbIpProtocol.ReadHeaderAsync(firstTls, ct)).Status.Should().Be(UsbIpProtocol.StatusOk);
        (await UsbIpProtocol.ReadHeaderAsync(secondTls, ct)).Status.Should().NotBe(UsbIpProtocol.StatusOk, "the device is in use by then");
    }

    [Fact]
    public async Task Leaving_the_server_role_ends_remote_hub_links_too()
    {
        var ct = TestContext.Current.CancellationToken;
        var (clientId, secret) = await RemoteAccess.CreateClientAsync(_server, "bench-5", ct);
        var token = await RemoteAccess.TokenAsync(_server, clientId, secret, ct);
        var (hub, closed, first) = await RemoteAccess.ConnectHubAsync(_server, token, ct);
        await using var link = hub;
        await first.WaitAsync(TimeSpan.FromSeconds(10), ct);

        using var ui = _server.CreateUiClient();
        (await ui.PatchAsJsonAsync("/api/settings", new { role = "client" }, ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);

        await closed.WaitAsync(TimeSpan.FromSeconds(10), ct);
    }

    [Fact]
    public async Task Leaving_the_server_role_is_confirmed_before_the_devices_are_given_back()
    {
        var ct = TestContext.Current.CancellationToken;
        using var gate = new ManualResetEventSlim(false);
        _kernel.UnshareGate = gate;
        using var ui = _server.CreateUiClient();
        try
        {
            // The unshare is blocked, and the settings change still returns.
            (await ui.PatchAsJsonAsync("/api/settings", new { role = "client" }, ct).WaitAsync(TimeSpan.FromSeconds(10), ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);
            _kernel.IsBound("1-1.4").Should().BeTrue("the teardown has not finished");
        }
        finally
        {
            gate.Set();
        }

        await Eventually(() => Task.FromResult(!_kernel.IsBound("1-1.4")), ct);
    }

    private async Task<string> TokenAsync(CancellationToken ct)
    {
        var (clientId, secret) = await RemoteAccess.CreateClientAsync(_server, "bench-2", ct);
        return await RemoteAccess.TokenAsync(_server, clientId, secret, ct);
    }

    private async Task<string> AnswerAsync(string token, CancellationToken ct)
    {
        var (session, answer) = await RemoteAccess.OpenUsbAsync(_server, token, ct);
        await session.DisposeAsync();
        return answer;
    }

    private static async Task Eventually(Func<Task<bool>> condition, CancellationToken ct)
    {
        var deadline = DateTime.UtcNow.AddSeconds(15);
        while (!await condition())
        {
            if (DateTime.UtcNow > deadline)
            {
                throw new TimeoutException("The condition did not hold in time.");
            }

            await Task.Delay(100, ct);
        }
    }
}
