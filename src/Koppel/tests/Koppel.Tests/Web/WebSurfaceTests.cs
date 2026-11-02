using System.Net;
using System.Net.Http.Json;
using System.Net.WebSockets;
using System.Text.Json;

namespace Koppel.Tests.Web;

/// <summary>
/// What is served on which port and role, and the guards on the UI's API. These run on real Kestrel,
/// because the port a request arrived on is what decides.
/// </summary>
public sealed class WebSurfaceTests : IDisposable
{
    private readonly List<IDisposable> _disposables = [];

    public void Dispose()
    {
        foreach (var disposable in _disposables)
        {
            disposable.Dispose();
        }
    }

    private KoppelFactory Host(string role = "server", Dictionary<string, string>? configuration = null)
    {
        var host = new KoppelFactory(kestrel: true, configuration: configuration, role: role);
        _disposables.Add(host);
        return host;
    }

    private HttpClient Own(HttpClient client)
    {
        _disposables.Add(client);
        return client;
    }

    [Fact]
    public async Task Ui_routes_answer_on_the_http_port_and_are_not_found_on_the_https_port()
    {
        var ct = TestContext.Current.CancellationToken;
        var host = Host();
        var ui = Own(host.CreateUiClient());
        var https = Own(host.CreateHttpsClient());

        foreach (var path in new[] { "/api/settings", "/api/server/identity", "/api/server/clients", "/api/server/pairing", "/api/system/info", "/api/activity" })
        {
            (await ui.GetAsync(path, ct)).StatusCode.Should().Be(HttpStatusCode.OK, path);
            (await https.GetAsync(path, ct)).StatusCode.Should().Be(HttpStatusCode.NotFound, path);
        }

        (await https.GetAsync("/", ct)).StatusCode.Should().Be(HttpStatusCode.NotFound);
        (await https.PostAsync("/api/hubs/ui/negotiate?negotiateVersion=1", null, ct)).StatusCode.Should().Be(HttpStatusCode.NotFound);
    }

    [Fact]
    public async Task Remote_routes_answer_on_the_https_port_and_are_not_found_on_the_http_port()
    {
        var ct = TestContext.Current.CancellationToken;
        var host = Host();
        var ui = Own(host.CreateUiClient());
        var https = Own(host.CreateHttpsClient());
        var form = new Dictionary<string, string> { ["grant_type"] = "client_credentials", ["client_id"] = "x", ["client_secret"] = "y" };

        (await ui.PostAsync("/connect/token", new FormUrlEncodedContent(form), ct)).StatusCode.Should().Be(HttpStatusCode.NotFound);
        (await https.PostAsync("/connect/token", new FormUrlEncodedContent(form), ct)).StatusCode.Should().Be(HttpStatusCode.Unauthorized, "the endpoint is there and refuses the credentials");
        (await ui.GetAsync("/api/remote/hello", ct)).StatusCode.Should().Be(HttpStatusCode.NotFound);
        (await https.GetAsync("/api/remote/hello", ct)).StatusCode.Should().Be(HttpStatusCode.Unauthorized);
        (await ui.PostAsJsonAsync("/api/pair", new { clientName = "x" }, ct)).StatusCode.Should().Be(HttpStatusCode.NotFound);
        (await https.PostAsJsonAsync("/api/pair", new { clientName = "x" }, ct)).StatusCode.Should().Be(HttpStatusCode.BadRequest);
        (await ui.PostAsync("/api/hubs/remote/negotiate?negotiateVersion=1", null, ct)).StatusCode.Should().Be(HttpStatusCode.NotFound);
    }

    [Fact]
    public async Task Healthz_answers_ok_on_the_http_port_without_the_header_in_any_role()
    {
        var ct = TestContext.Current.CancellationToken;
        foreach (var role in new[] { "server", "client" })
        {
            var host = Host(role);
            var plain = Own(host.CreatePlainClient());

            using var response = await plain.GetAsync("/healthz", ct);

            response.StatusCode.Should().Be(HttpStatusCode.OK);
            (await response.Content.ReadAsStringAsync(ct)).Should().Be("ok");
        }
    }

    [Fact]
    public async Task Server_surfaces_are_not_found_in_the_client_role()
    {
        var ct = TestContext.Current.CancellationToken;
        var host = Host("client");
        var ui = Own(host.CreateUiClient());
        var https = Own(host.CreateHttpsClient());

        foreach (var path in new[] { "/api/server/identity", "/api/server/clients", "/api/server/pairing", "/api/server/devices" })
        {
            (await ui.GetAsync(path, ct)).StatusCode.Should().Be(HttpStatusCode.NotFound, path);
        }

        (await https.PostAsync("/connect/token", new FormUrlEncodedContent(new Dictionary<string, string>()), ct)).StatusCode.Should().Be(HttpStatusCode.NotFound);
        (await https.GetAsync("/api/remote/hello", ct)).StatusCode.Should().Be(HttpStatusCode.NotFound);
        (await https.PostAsJsonAsync("/api/pair", new { clientName = "x" }, ct)).StatusCode.Should().Be(HttpStatusCode.NotFound);
        (await https.PostAsync("/api/hubs/remote/negotiate?negotiateVersion=1", null, ct)).StatusCode.Should().Be(HttpStatusCode.NotFound);

        // What the UI shows in every role keeps working.
        foreach (var path in new[] { "/api/settings", "/api/system/info", "/api/system/checks", "/api/activity", "/api/client/connection", "/api/client/devices" })
        {
            (await ui.GetAsync(path, ct)).StatusCode.Should().Be(HttpStatusCode.OK, path);
        }
    }

    [Fact]
    public async Task Client_surfaces_are_not_found_in_the_server_role()
    {
        var ct = TestContext.Current.CancellationToken;
        var host = Host("server");
        var ui = Own(host.CreateUiClient());

        foreach (var path in new[] { "/api/client/connection", "/api/client/devices", "/api/client/discovery", "/api/client/pairing" })
        {
            (await ui.GetAsync(path, ct)).StatusCode.Should().Be(HttpStatusCode.NotFound, path);
        }

        (await ui.GetAsync("/api/settings", ct)).StatusCode.Should().Be(HttpStatusCode.OK);
    }

    [Fact]
    public async Task A_role_change_takes_effect_on_the_next_request()
    {
        var ct = TestContext.Current.CancellationToken;
        var host = Host("server");
        var ui = Own(host.CreateUiClient());
        (await ui.GetAsync("/api/server/identity", ct)).StatusCode.Should().Be(HttpStatusCode.OK);

        (await ui.PatchAsJsonAsync("/api/settings", new { role = "client" }, ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);
        (await ui.GetAsync("/api/server/identity", ct)).StatusCode.Should().Be(HttpStatusCode.NotFound);
        (await ui.GetAsync("/api/client/connection", ct)).StatusCode.Should().Be(HttpStatusCode.OK);

        (await ui.PatchAsJsonAsync("/api/settings", new { role = "both" }, ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);
        (await ui.GetAsync("/api/server/identity", ct)).StatusCode.Should().Be(HttpStatusCode.OK);
        (await ui.GetAsync("/api/client/connection", ct)).StatusCode.Should().Be(HttpStatusCode.OK);
    }

    [Fact]
    public async Task A_change_without_the_ui_header_is_refused_and_a_read_is_not()
    {
        var ct = TestContext.Current.CancellationToken;
        var host = Host();
        var plain = Own(host.CreatePlainClient());
        var ui = Own(host.CreateUiClient());

        (await plain.GetAsync("/api/settings", ct)).StatusCode.Should().Be(HttpStatusCode.OK);
        (await plain.PatchAsJsonAsync("/api/settings", new { shareOnStartup = false }, ct)).StatusCode.Should().Be(HttpStatusCode.Forbidden);
        (await plain.PostAsync("/api/server/clients/nobody/rotate", null, ct)).StatusCode.Should().Be(HttpStatusCode.Forbidden);
        (await ui.PatchAsJsonAsync("/api/settings", new { shareOnStartup = false }, ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);
    }

    [Fact]
    public async Task A_request_from_another_origin_is_refused_and_the_same_origin_is_not()
    {
        var ct = TestContext.Current.CancellationToken;
        var host = Host();
        var ui = Own(host.CreateUiClient());

        async Task<HttpStatusCode> Get(string origin, string path = "/api/settings")
        {
            using var request = new HttpRequestMessage(HttpMethod.Get, path);
            request.Headers.Add("Origin", origin);
            using var response = await ui.SendAsync(request, ct);
            return response.StatusCode;
        }

        (await Get($"http://localhost:{host.HttpPort}")).Should().Be(HttpStatusCode.OK);
        (await Get("https://evil.example")).Should().Be(HttpStatusCode.Forbidden);
        (await Get($"http://localhost:{host.HttpPort + 1}")).Should().Be(HttpStatusCode.Forbidden, "another port is another origin");
        (await Get($"https://localhost:{host.HttpPort}")).Should().Be(HttpStatusCode.Forbidden, "another scheme is another origin");
        (await Get("null")).Should().Be(HttpStatusCode.Forbidden);
        (await Get("https://evil.example", "/api/hubs/ui/negotiate?negotiateVersion=1")).Should().Be(HttpStatusCode.Forbidden);
    }

    [Fact]
    public async Task The_hub_refuses_a_websocket_upgrade_from_another_origin()
    {
        var ct = TestContext.Current.CancellationToken;
        var host = Host();
        using var socket = new ClientWebSocket();
        socket.Options.SetRequestHeader("Origin", "https://evil.example");

        var connect = () => socket.ConnectAsync(new Uri($"ws://localhost:{host.HttpPort}/api/hubs/ui"), ct);

        (await connect.Should().ThrowAsync<WebSocketException>()).Which.Message.Should().Contain("403");
    }

    [Fact]
    public async Task Every_response_carries_the_security_headers()
    {
        var ct = TestContext.Current.CancellationToken;
        var host = Host();
        var ui = Own(host.CreateUiClient());
        var https = Own(host.CreateHttpsClient());

        foreach (var response in new[] { await ui.GetAsync("/api/settings", ct), await ui.GetAsync("/healthz", ct), await https.GetAsync("/api/settings", ct), await ui.GetAsync("/api/nope-not-a-route", ct) })
        {
            using (response)
            {
                response.Headers.GetValues("X-Frame-Options").Should().Equal("DENY");
                response.Headers.GetValues("Content-Security-Policy").Should().Equal("frame-ancestors 'none'");
                response.Headers.GetValues("X-Content-Type-Options").Should().Equal("nosniff");
                response.Headers.GetValues("Referrer-Policy").Should().Equal("no-referrer");
            }
        }
    }

    [Fact]
    public async Task The_token_endpoint_and_pairing_are_rate_limited_by_address()
    {
        var ct = TestContext.Current.CancellationToken;
        var host = Host(configuration: new() { ["RateLimits:TokenPerMinute"] = "3", ["RateLimits:PairPerMinute"] = "4" });
        var https = Own(host.CreateHttpsClient());
        var form = new Dictionary<string, string> { ["grant_type"] = "client_credentials", ["client_id"] = "x", ["client_secret"] = "y" };

        var tokens = new List<HttpStatusCode>();
        for (var i = 0; i < 5; i++)
        {
            tokens.Add((await https.PostAsync("/connect/token", new FormUrlEncodedContent(form), ct)).StatusCode);
        }

        tokens.Should().Equal(HttpStatusCode.Unauthorized, HttpStatusCode.Unauthorized, HttpStatusCode.Unauthorized, HttpStatusCode.TooManyRequests, HttpStatusCode.TooManyRequests);

        var pairs = new List<HttpStatusCode>();
        for (var i = 0; i < 5; i++)
        {
            pairs.Add((await https.GetAsync("/api/pair/nothing", ct)).StatusCode);
        }

        pairs.Should().Equal(HttpStatusCode.NotFound, HttpStatusCode.NotFound, HttpStatusCode.NotFound, HttpStatusCode.NotFound, HttpStatusCode.TooManyRequests);

        // The UI port is not limited.
        var ui = Own(host.CreateUiClient());
        for (var i = 0; i < 10; i++)
        {
            (await ui.GetAsync("/api/settings", ct)).StatusCode.Should().Be(HttpStatusCode.OK);
        }
    }

    [Fact]
    public async Task The_http_port_binds_to_the_configured_address()
    {
        var ct = TestContext.Current.CancellationToken;
        var host = Host(configuration: new() { ["Ports:HttpAddress"] = "127.0.0.1" });
        var ui = Own(host.CreateUiClient());
        ui.BaseAddress = new Uri($"http://127.0.0.1:{host.HttpPort}");

        (await ui.GetAsync("/api/settings", ct)).StatusCode.Should().Be(HttpStatusCode.OK);
    }

    [Fact]
    public async Task A_settings_patch_that_changes_nothing_records_no_activity()
    {
        var ct = TestContext.Current.CancellationToken;
        var host = Host();
        var ui = Own(host.CreateUiClient());

        (await ui.PatchAsJsonAsync("/api/settings", new { role = "server", shareOnStartup = true }, ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);
        (await ui.GetFromJsonAsync<JsonElement>("/api/activity", ct)).GetArrayLength().Should().Be(0);

        (await ui.PatchAsJsonAsync("/api/settings", new { shareOnStartup = false }, ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);
        var activity = await ui.GetFromJsonAsync<JsonElement>("/api/activity", ct);
        activity.GetArrayLength().Should().Be(1);
        activity[0].GetRawText().Should().Contain("Share saved devices on startup turned off").And.NotContain("Role set");
    }

    [Fact]
    public async Task The_connection_test_answers_with_a_fixed_outcome_and_no_exception_text()
    {
        var ct = TestContext.Current.CancellationToken;
        var host = Host("client");
        var ui = Own(host.CreateUiClient());
        var deadPort = KoppelFactory.FreePort();
        (await ui.PutAsJsonAsync("/api/client/connection", new
        {
            serverUrl = $"https://127.0.0.1:{deadPort}",
            serverFingerprint = new string('A', 64),
            clientId = "client-1",
            clientSecret = "secret",
        }, ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);

        using var response = await ui.PostAsync("/api/client/connection/test", null, ct);

        var result = await response.Content.ReadFromJsonAsync<JsonElement>(ct);
        result.GetProperty("ok").GetBoolean().Should().BeFalse();
        result.GetProperty("outcome").GetString().Should().Be("unreachable");
        var message = result.GetProperty("message").GetString()!;
        message.Should().NotContain(deadPort.ToString(System.Globalization.CultureInfo.InvariantCulture)).And.NotContain("refused").And.NotContain("Exception");
    }
}
