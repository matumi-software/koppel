using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Koppel.Core.Security;
using Koppel.Login;
using Microsoft.AspNetCore.Authentication.Cookies;
using Microsoft.AspNetCore.Http;
using Microsoft.AspNetCore.SignalR.Client;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Options;

namespace Koppel.Tests.Login;

/// <summary>The UI's sign-in, end to end. Hosts are real Kestrel, so the port a request arrives on decides what it reaches.</summary>
public sealed class UiLoginTests : IDisposable
{
    private const string NewPassword = "another long password";

    private readonly List<IDisposable> _disposables = [];

    public void Dispose()
    {
        foreach (var disposable in _disposables)
        {
            disposable.Dispose();
        }
    }

    private KoppelFactory Host()
    {
        var host = new KoppelFactory(kestrel: true);
        _disposables.Add(host);
        return host;
    }

    private HttpClient Own(HttpClient client)
    {
        _disposables.Add(client);
        return client;
    }

    private static string CookieOf(HttpResponseMessage response) => response.Headers.GetValues("Set-Cookie").Single().Split(';')[0];

    private HttpClient WithCookie(KoppelFactory host, string cookie)
    {
        var client = Own(host.CreateAnonymousUiClient());
        client.DefaultRequestHeaders.Add("Cookie", cookie);
        return client;
    }

    private static async Task<string> SetPasswordAsync(KoppelFactory host)
    {
        using var client = host.CreateAnonymousUiClient();
        using var response = await client.PostAsJsonAsync("/api/auth/setup", new { setupCode = host.SetupCode, password = KoppelFactory.Password }, TestContext.Current.CancellationToken);
        response.StatusCode.Should().Be(HttpStatusCode.NoContent);
        return CookieOf(response);
    }

    private static async Task<string?> ErrorOf(HttpResponseMessage response, CancellationToken ct) =>
        (await response.Content.ReadFromJsonAsync<JsonElement>(ct)).GetProperty("error").GetString();

    [Fact]
    public async Task The_ui_api_and_hub_answer_401_without_a_session()
    {
        var ct = TestContext.Current.CancellationToken;
        var host = Host();
        var anonymous = Own(host.CreateAnonymousUiClient());

        foreach (var path in new[] { "/api/settings", "/api/server/identity", "/api/server/clients", "/api/system/info", "/api/activity", "/api/not-a-route-yet" })
        {
            (await anonymous.GetAsync(path, ct)).StatusCode.Should().Be(HttpStatusCode.Unauthorized, path);
        }

        (await anonymous.PatchAsJsonAsync("/api/settings", new { shareOnStartup = false }, ct)).StatusCode.Should().Be(HttpStatusCode.Unauthorized);
        (await anonymous.PostAsync("/api/auth/logout", null, ct)).StatusCode.Should().Be(HttpStatusCode.Unauthorized);
        (await anonymous.PostAsJsonAsync("/api/auth/password", new { current = "x", next = "y" }, ct)).StatusCode.Should().Be(HttpStatusCode.Unauthorized);
        (await anonymous.PostAsync("/api/hubs/ui/negotiate?negotiateVersion=1", null, ct)).StatusCode.Should().Be(HttpStatusCode.Unauthorized);

        await using var hub = host.BuildAnonymousUiHub();
        var start = () => hub.StartAsync(ct);
        (await start.Should().ThrowAsync<HttpRequestException>()).Which.StatusCode.Should().Be(HttpStatusCode.Unauthorized);
    }

    [Fact]
    public async Task Anonymous_routes_stay_open()
    {
        var ct = TestContext.Current.CancellationToken;
        var host = Host();
        var anonymous = Own(host.CreateAnonymousUiClient());

        (await anonymous.GetAsync("/healthz", ct)).StatusCode.Should().Be(HttpStatusCode.OK);
        (await anonymous.GetAsync("/", ct)).StatusCode.Should().NotBe(HttpStatusCode.Unauthorized, "the SPA must load to show the login screen");
        (await anonymous.GetAsync("/some/spa/route", ct)).StatusCode.Should().NotBe(HttpStatusCode.Unauthorized);
        var state = await anonymous.GetFromJsonAsync<JsonElement>("/api/auth/state", ct);
        state.GetProperty("signedIn").GetBoolean().Should().BeFalse();
        state.GetProperty("passwordSet").GetBoolean().Should().BeFalse();
        state.GetProperty("machineName").GetString().Should().Be(Environment.MachineName);
    }

    [Fact]
    public async Task The_remote_surfaces_are_unaffected_and_ignore_the_cookie()
    {
        var ct = TestContext.Current.CancellationToken;
        var host = Host();
        var cookie = await SetPasswordAsync(host);
        var https = Own(host.CreateHttpsClient());
        https.DefaultRequestHeaders.Add("Cookie", cookie);
        var form = new Dictionary<string, string> { ["grant_type"] = "client_credentials", ["client_id"] = "x", ["client_secret"] = "y" };

        (await https.PostAsync("/connect/token", new FormUrlEncodedContent(form), ct)).StatusCode.Should().Be(HttpStatusCode.Unauthorized);
        (await https.GetAsync("/api/remote/hello", ct)).StatusCode.Should().Be(HttpStatusCode.Unauthorized, "a UI session is not a bearer token");
        (await https.PostAsJsonAsync("/api/pair", new { clientName = "x" }, ct)).StatusCode.Should().Be(HttpStatusCode.BadRequest);
        (await https.GetAsync("/api/auth/state", ct)).StatusCode.Should().Be(HttpStatusCode.NotFound, "sign-in is the UI port's");
    }

    [Fact]
    public async Task Setup_needs_the_code_and_works_once()
    {
        var ct = TestContext.Current.CancellationToken;
        var host = Host();
        var anonymous = Own(host.CreateAnonymousUiClient());
        var code = host.SetupCode;
        code.Should().MatchRegex("^[A-HJKMNP-Z2-9]{8}$");

        using var wrong = await anonymous.PostAsJsonAsync("/api/auth/setup", new { setupCode = "AAAA-AAAA", password = KoppelFactory.Password }, ct);
        wrong.StatusCode.Should().Be(HttpStatusCode.Forbidden);
        wrong.Headers.Contains("Set-Cookie").Should().BeFalse();
        using var missing = await anonymous.PostAsJsonAsync("/api/auth/setup", new { password = KoppelFactory.Password }, ct);
        missing.StatusCode.Should().Be(HttpStatusCode.Forbidden);
        (await anonymous.GetAsync("/api/settings", ct)).StatusCode.Should().Be(HttpStatusCode.Unauthorized);

        // The code as it is shown, dash and lower case included.
        var shown = $"{code[..4]}-{code[4..]}".ToLowerInvariant();
        using var right = await anonymous.PostAsJsonAsync("/api/auth/setup", new { setupCode = shown, password = KoppelFactory.Password }, ct);
        right.StatusCode.Should().Be(HttpStatusCode.NoContent);
        var signedIn = WithCookie(host, CookieOf(right));
        (await signedIn.GetAsync("/api/settings", ct)).StatusCode.Should().Be(HttpStatusCode.OK);

        using var again = await anonymous.PostAsJsonAsync("/api/auth/setup", new { setupCode = code, password = NewPassword }, ct);
        again.StatusCode.Should().Be(HttpStatusCode.Conflict);
        host.Services.GetRequiredService<UiAuthStore>().SetupCode.Should().BeNull();
    }

    [Fact]
    public async Task Setup_refuses_a_weak_password_and_keeps_the_code()
    {
        var ct = TestContext.Current.CancellationToken;
        var host = Host();
        var anonymous = Own(host.CreateAnonymousUiClient());

        foreach (var weak in new[] { "short", "KOPPEL", "Password", "" })
        {
            using var response = await anonymous.PostAsJsonAsync("/api/auth/setup", new { setupCode = host.SetupCode, password = weak }, ct);
            response.StatusCode.Should().Be(HttpStatusCode.BadRequest, weak);
        }

        host.Services.GetRequiredService<UiAuthStore>().HasPassword.Should().BeFalse();
    }

    [Fact]
    public async Task Wrong_setup_codes_are_rate_limited()
    {
        var ct = TestContext.Current.CancellationToken;
        var host = Host();
        var anonymous = Own(host.CreateAnonymousUiClient());
        var code = host.SetupCode;

        for (var i = 0; i < UiLoginThrottle.FreeFailures; i++)
        {
            (await anonymous.PostAsJsonAsync("/api/auth/setup", new { setupCode = "AAAA-AAAA", password = KoppelFactory.Password }, ct)).StatusCode.Should().Be(HttpStatusCode.Forbidden);
        }

        using var locked = await anonymous.PostAsJsonAsync("/api/auth/setup", new { setupCode = code, password = KoppelFactory.Password }, ct);
        locked.StatusCode.Should().Be(HttpStatusCode.TooManyRequests, "the right code does not help during a lock");
        locked.Headers.RetryAfter!.Delta.Should().BeGreaterThan(TimeSpan.Zero);
    }

    [Fact]
    public async Task The_setup_code_is_written_to_a_file_and_removed_once_a_password_is_set()
    {
        var host = Host();
        var file = host.ConfigFile("setup-code.txt");

        File.Exists(file).Should().BeTrue();
        File.ReadAllText(file).Trim().Replace("-", "").Should().Be(host.SetupCode);

        await SetPasswordAsync(host);

        File.Exists(file).Should().BeFalse();
        File.Exists(host.ConfigFile("ui-auth.json")).Should().BeTrue();
        File.ReadAllText(host.ConfigFile("ui-auth.json")).Should().NotContain(KoppelFactory.Password, "the file is encrypted");
    }

    [Fact]
    public async Task Login_works_with_the_password_and_not_with_another()
    {
        var ct = TestContext.Current.CancellationToken;
        var host = Host();
        var anonymous = Own(host.CreateAnonymousUiClient());

        using var before = await anonymous.PostAsJsonAsync("/api/auth/login", new { password = KoppelFactory.Password }, ct);
        before.StatusCode.Should().Be(HttpStatusCode.Conflict, "nothing to sign in to before setup");
        await SetPasswordAsync(host);

        using var wrong = await anonymous.PostAsJsonAsync("/api/auth/login", new { password = "not the password" }, ct);
        wrong.StatusCode.Should().Be(HttpStatusCode.Unauthorized);
        wrong.Headers.Contains("Set-Cookie").Should().BeFalse();
        using var right = await anonymous.PostAsJsonAsync("/api/auth/login", new { password = KoppelFactory.Password }, ct);
        right.StatusCode.Should().Be(HttpStatusCode.NoContent);

        var signedIn = WithCookie(host, CookieOf(right));
        var state = await signedIn.GetFromJsonAsync<JsonElement>("/api/auth/state", ct);
        state.GetProperty("signedIn").GetBoolean().Should().BeTrue();
        state.GetProperty("passwordSet").GetBoolean().Should().BeTrue();
        (await signedIn.GetAsync("/api/settings", ct)).StatusCode.Should().Be(HttpStatusCode.OK);
    }

    [Fact]
    public async Task Repeated_wrong_passwords_lock_the_caller_out_and_are_logged_in_the_activity_log()
    {
        var ct = TestContext.Current.CancellationToken;
        var host = Host();
        await SetPasswordAsync(host);
        var anonymous = Own(host.CreateAnonymousUiClient());

        for (var i = 0; i < UiLoginThrottle.FreeFailures; i++)
        {
            (await anonymous.PostAsJsonAsync("/api/auth/login", new { password = "wrong" + i }, ct)).StatusCode.Should().Be(HttpStatusCode.Unauthorized);
        }

        using var locked = await anonymous.PostAsJsonAsync("/api/auth/login", new { password = KoppelFactory.Password }, ct);
        locked.StatusCode.Should().Be(HttpStatusCode.TooManyRequests, "the right password does not help during a lock");
        locked.Headers.RetryAfter!.Delta.Should().BeGreaterThan(TimeSpan.FromSeconds(30));
        (await ErrorOf(locked, ct)).Should().Contain("Wait");

        var ui = Own(host.CreateUiClient());
        var activity = await ui.GetFromJsonAsync<JsonElement>("/api/activity?max=50", ct);
        activity.EnumerateArray().Count(e => e.GetProperty("reason").GetString() == "signInFailed" && e.GetProperty("outcome").GetString() == "failed")
            .Should().Be(UiLoginThrottle.FreeFailures, "the locked attempt is not logged again");
    }

    [Fact]
    public async Task The_session_cookie_is_http_only_strict_and_named_koppel_ui()
    {
        var ct = TestContext.Current.CancellationToken;
        var host = Host();
        var anonymous = Own(host.CreateAnonymousUiClient());

        using var response = await anonymous.PostAsJsonAsync("/api/auth/setup", new { setupCode = host.SetupCode, password = KoppelFactory.Password }, ct);

        var cookie = response.Headers.GetValues("Set-Cookie").Single();
        cookie.Should().StartWith("koppel.ui=");
        cookie.ToLowerInvariant().Should().Contain("httponly").And.Contain("samesite=strict").And.NotContain("secure", "the request was plain HTTP");
        cookie.ToLowerInvariant().Should().NotContain("expires=", "a session cookie; the ticket carries the 12 hours");

        var options = host.Services.GetRequiredService<IOptionsMonitor<CookieAuthenticationOptions>>().Get(UiSession.Scheme);
        options.Cookie.HttpOnly.Should().BeTrue();
        options.Cookie.SameSite.Should().Be(SameSiteMode.Strict);
        options.Cookie.SecurePolicy.Should().Be(CookieSecurePolicy.SameAsRequest, "Secure whenever the request is HTTPS");
        options.SlidingExpiration.Should().BeTrue();
        options.ExpireTimeSpan.Should().Be(TimeSpan.FromHours(12));
    }

    [Fact]
    public async Task Changing_the_password_signs_out_other_sessions_and_keeps_this_one()
    {
        var ct = TestContext.Current.CancellationToken;
        var host = Host();
        var first = await SetPasswordAsync(host);
        var other = Own(host.CreateAnonymousUiClient());
        using var otherLogin = await other.PostAsJsonAsync("/api/auth/login", new { password = KoppelFactory.Password }, ct);
        var second = CookieOf(otherLogin);
        await using var otherHub = await ConnectHubAsync(host, second, ct);
        var otherClosed = new TaskCompletionSource();
        otherHub.Closed += _ => { otherClosed.TrySetResult(); return Task.CompletedTask; };
        await using var thisHub = await ConnectHubAsync(host, first, ct);
        var thisClosed = new TaskCompletionSource();
        thisHub.Closed += _ => { thisClosed.TrySetResult(); return Task.CompletedTask; };

        var changing = WithCookie(host, first);
        using (var wrong = await changing.PostAsJsonAsync("/api/auth/password", new { current = "not it", next = NewPassword }, ct))
        {
            wrong.StatusCode.Should().Be(HttpStatusCode.BadRequest);
        }

        using (var weak = await changing.PostAsJsonAsync("/api/auth/password", new { current = KoppelFactory.Password, next = "short" }, ct))
        {
            weak.StatusCode.Should().Be(HttpStatusCode.BadRequest);
        }

        (await WithCookie(host, second).GetAsync("/api/settings", ct)).StatusCode.Should().Be(HttpStatusCode.OK, "nothing changed yet");

        using var changed = await changing.PostAsJsonAsync("/api/auth/password", new { current = KoppelFactory.Password, next = NewPassword }, ct);

        changed.StatusCode.Should().Be(HttpStatusCode.NoContent);
        var renewed = CookieOf(changed);
        (await WithCookie(host, second).GetAsync("/api/settings", ct)).StatusCode.Should().Be(HttpStatusCode.Unauthorized, "the other session is signed out");
        (await WithCookie(host, first).GetAsync("/api/settings", ct)).StatusCode.Should().Be(HttpStatusCode.Unauthorized, "the old cookie of this session carries the old stamp");
        (await WithCookie(host, renewed).GetAsync("/api/settings", ct)).StatusCode.Should().Be(HttpStatusCode.OK, "this session gets a cookie for the new stamp");
        await otherClosed.Task.WaitAsync(TimeSpan.FromSeconds(10), ct);
        thisClosed.Task.IsCompleted.Should().BeFalse("the session that changed the password keeps its hub");

        var anonymous = Own(host.CreateAnonymousUiClient());
        (await anonymous.PostAsJsonAsync("/api/auth/login", new { password = KoppelFactory.Password }, ct)).StatusCode.Should().Be(HttpStatusCode.Unauthorized);
        (await anonymous.PostAsJsonAsync("/api/auth/login", new { password = NewPassword }, ct)).StatusCode.Should().Be(HttpStatusCode.NoContent);
    }

    [Fact]
    public async Task Signing_out_ends_the_session_and_closes_its_hub()
    {
        var ct = TestContext.Current.CancellationToken;
        var host = Host();
        var cookie = await SetPasswordAsync(host);
        await using var hub = await ConnectHubAsync(host, cookie, ct);
        var closed = new TaskCompletionSource();
        hub.Closed += _ => { closed.TrySetResult(); return Task.CompletedTask; };
        var client = WithCookie(host, cookie);

        using var response = await client.PostAsync("/api/auth/logout", null, ct);

        response.StatusCode.Should().Be(HttpStatusCode.NoContent);
        await closed.Task.WaitAsync(TimeSpan.FromSeconds(10), ct);
    }

    [Fact]
    public async Task Resetting_the_password_clears_it_ends_sessions_and_issues_a_new_setup_code()
    {
        var ct = TestContext.Current.CancellationToken;
        var host = Host();
        var cookie = await SetPasswordAsync(host);
        var paths = host.Services.GetRequiredService<KoppelPaths>();
        var store = host.Services.GetRequiredService<UiAuthStore>();
        var output = new StringWriter();

        var exit = await UiPasswordReset.RunAsync(paths, host.Services.GetRequiredService<SecretBox>(), output);

        exit.Should().Be(0);
        output.ToString().Should().Contain("setup code").And.Contain(paths.SetupCodeFile);

        // The running host sees the file change.
        var deadline = DateTime.UtcNow.AddSeconds(15);
        while (store.HasPassword && DateTime.UtcNow < deadline)
        {
            await Task.Delay(100, ct);
        }

        store.HasPassword.Should().BeFalse();
        store.SetupCode.Should().NotBeNull();
        File.Exists(host.ConfigFile("setup-code.txt")).Should().BeTrue();
        (await WithCookie(host, cookie).GetAsync("/api/settings", ct)).StatusCode.Should().Be(HttpStatusCode.Unauthorized);
        using var login = await Own(host.CreateAnonymousUiClient()).PostAsJsonAsync("/api/auth/login", new { password = KoppelFactory.Password }, ct);
        login.StatusCode.Should().Be(HttpStatusCode.Conflict);

        await SetPasswordAsync(host);
        File.Exists(host.ConfigFile("setup-code.txt")).Should().BeFalse();
    }

    [Fact]
    public async Task Reset_works_on_a_machine_that_never_ran_and_on_an_unreadable_file()
    {
        var directory = Path.Combine(Path.GetTempPath(), "koppel-tests", Guid.NewGuid().ToString("N"));
        try
        {
            var paths = new KoppelPaths(Path.Combine(directory, "config"), Path.Combine(directory, "keys"));
            paths.Prepare();
            using var box = new SecretBox(paths.KeysDirectory, "Koppel.Secrets.v1");

            (await UiPasswordReset.RunAsync(paths, box, new StringWriter())).Should().Be(0);
            File.Exists(paths.UiAuthFile).Should().BeTrue();

            await File.WriteAllTextAsync(paths.UiAuthFile, "not encrypted at all", TestContext.Current.CancellationToken);
            (await UiPasswordReset.RunAsync(paths, box, new StringWriter())).Should().Be(0);
            using var store = new UiAuthStore(paths, box, Microsoft.Extensions.Logging.Abstractions.NullLogger<UiAuthStore>.Instance);
            await store.StartAsync(TestContext.Current.CancellationToken);
            store.HasPassword.Should().BeFalse();
            await store.StopAsync(TestContext.Current.CancellationToken);
        }
        finally
        {
            Directory.Delete(directory, recursive: true);
        }
    }

    [Fact]
    public void Reset_is_asked_for_by_its_flag()
    {
        UiPasswordReset.Requested(["--reset-password"]).Should().BeTrue();
        UiPasswordReset.Requested(["--urls", "x"]).Should().BeFalse();
    }

    private static async Task<HubConnection> ConnectHubAsync(KoppelFactory host, string cookie, CancellationToken ct)
    {
        var hub = new HubConnectionBuilder()
            .WithUrl($"http://localhost:{host.HttpPort}/api/hubs/ui", options => options.Headers["Cookie"] = cookie)
            .Build();
        await hub.StartAsync(ct);
        return hub;
    }
}
