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

    private KoppelFactory Host(ManualTimeProvider? clock = null, IReadOnlyDictionary<string, string>? configuration = null)
    {
        var host = new KoppelFactory(kestrel: true, configuration: configuration, services: clock is null ? null : s => s.AddSingleton<TimeProvider>(clock));
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
        using var response = await client.PostAsJsonAsync("/api/auth/setup", new { password = KoppelFactory.Password }, TestContext.Current.CancellationToken);
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
        state.GetProperty("setupOpen").GetBoolean().Should().BeTrue();
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

        (await https.PostAsync("/skakel/v1/token", new FormUrlEncodedContent(form), ct)).StatusCode.Should().Be(HttpStatusCode.Unauthorized);
        (await https.GetAsync("/skakel/v1/hello", ct)).StatusCode.Should().Be(HttpStatusCode.Unauthorized, "a UI session is not a bearer token");
        (await https.PostAsJsonAsync("/skakel/v1/pair", new { clientName = "x" }, ct)).StatusCode.Should().Be(HttpStatusCode.BadRequest);
        (await https.GetAsync("/api/auth/state", ct)).StatusCode.Should().Be(HttpStatusCode.NotFound, "sign-in is the UI port's");
    }

    [Fact]
    public async Task Setup_takes_only_the_password_while_open_and_works_once()
    {
        var ct = TestContext.Current.CancellationToken;
        var clock = new ManualTimeProvider();
        var host = Host(clock);
        var anonymous = Own(host.CreateAnonymousUiClient());
        var before = await anonymous.GetFromJsonAsync<JsonElement>("/api/auth/state", ct);
        before.GetProperty("setupOpen").GetBoolean().Should().BeTrue();
        before.GetProperty("setupUntil").GetDateTimeOffset().Should().Be(clock.GetUtcNow() + UiSetupWindow.DefaultLength);
        clock.Advance(UiSetupWindow.DefaultLength - TimeSpan.FromSeconds(1));

        using var right = await anonymous.PostAsJsonAsync("/api/auth/setup", new { password = KoppelFactory.Password }, ct);

        right.StatusCode.Should().Be(HttpStatusCode.NoContent);
        var signedIn = WithCookie(host, CookieOf(right));
        (await signedIn.GetAsync("/api/settings", ct)).StatusCode.Should().Be(HttpStatusCode.OK);
        var after = await anonymous.GetFromJsonAsync<JsonElement>("/api/auth/state", ct);
        after.GetProperty("passwordSet").GetBoolean().Should().BeTrue();
        after.GetProperty("setupOpen").GetBoolean().Should().BeFalse();
        after.GetProperty("setupUntil").ValueKind.Should().Be(JsonValueKind.Null);

        using var again = await anonymous.PostAsJsonAsync("/api/auth/setup", new { password = NewPassword }, ct);
        again.StatusCode.Should().Be(HttpStatusCode.Conflict);
        (await ErrorOf(again, ct)).Should().Be("A password is already set.");
        (await anonymous.PostAsJsonAsync("/api/auth/login", new { password = NewPassword }, ct)).StatusCode.Should().Be(HttpStatusCode.Unauthorized, "the second setup changed nothing");
    }

    [Fact]
    public async Task Setup_is_refused_once_the_window_has_closed()
    {
        var ct = TestContext.Current.CancellationToken;
        var clock = new ManualTimeProvider();
        var host = Host(clock);
        var anonymous = Own(host.CreateAnonymousUiClient());
        clock.Advance(UiSetupWindow.DefaultLength);

        using var late = await anonymous.PostAsJsonAsync("/api/auth/setup", new { password = KoppelFactory.Password }, ct);

        late.StatusCode.Should().Be(HttpStatusCode.Forbidden);
        late.Headers.Contains("Set-Cookie").Should().BeFalse();
        (await ErrorOf(late, ct)).Should().Contain("timed out").And.Contain("10 minutes").And.Contain("Restart Koppel");
        host.Services.GetRequiredService<UiAuthStore>().HasPassword.Should().BeFalse();
        var state = await anonymous.GetFromJsonAsync<JsonElement>("/api/auth/state", ct);
        state.GetProperty("passwordSet").GetBoolean().Should().BeFalse();
        state.GetProperty("setupOpen").GetBoolean().Should().BeFalse();
        state.GetProperty("setupUntil").GetDateTimeOffset().Should().Be(clock.GetUtcNow(), "the UI says when it closed");
    }

    [Fact]
    public async Task The_setup_window_follows_its_setting()
    {
        var ct = TestContext.Current.CancellationToken;
        var clock = new ManualTimeProvider();
        var host = Host(clock, new Dictionary<string, string> { [UiSetupWindow.Setting] = "2" });
        var anonymous = Own(host.CreateAnonymousUiClient());

        var state = await anonymous.GetFromJsonAsync<JsonElement>("/api/auth/state", ct);
        state.GetProperty("setupUntil").GetDateTimeOffset().Should().Be(clock.GetUtcNow() + TimeSpan.FromMinutes(2));

        clock.Advance(TimeSpan.FromMinutes(2));
        using var late = await anonymous.PostAsJsonAsync("/api/auth/setup", new { password = KoppelFactory.Password }, ct);
        late.StatusCode.Should().Be(HttpStatusCode.Forbidden);
        (await ErrorOf(late, ct)).Should().Contain("2 minutes");
    }

    [Theory]
    [InlineData(null, 10)]
    [InlineData("", 10)]
    [InlineData(" 5 ", 5)]
    [InlineData("1440", 1440)]
    public void The_setup_window_setting_is_whole_minutes(string? setting, int minutes) =>
        UiSetupWindow.Resolve(setting).Should().Be(TimeSpan.FromMinutes(minutes));

    [Theory]
    [InlineData("0")]
    [InlineData("-1")]
    [InlineData("1.5")]
    [InlineData("1441")]
    [InlineData("ten")]
    public void A_bad_setup_window_setting_stops_the_start_and_names_it(string setting)
    {
        var resolve = () => UiSetupWindow.Resolve(setting);
        resolve.Should().Throw<InvalidOperationException>().WithMessage($"*{UiSetupWindow.Setting}*");
    }

    [Fact]
    public async Task Setup_refuses_a_weak_password_and_stays_open()
    {
        var ct = TestContext.Current.CancellationToken;
        var host = Host();
        var anonymous = Own(host.CreateAnonymousUiClient());

        foreach (var weak in new[] { "short", "KOPPEL", "Password", "" })
        {
            using var response = await anonymous.PostAsJsonAsync("/api/auth/setup", new { password = weak }, ct);
            response.StatusCode.Should().Be(HttpStatusCode.BadRequest, weak);
        }

        host.Services.GetRequiredService<UiAuthStore>().HasPassword.Should().BeFalse();
        host.Services.GetRequiredService<UiAuthStore>().SetupOpen.Should().BeTrue();
    }

    [Fact]
    public async Task Many_parallel_wrong_logins_run_at_most_five_password_checks_per_lock_window()
    {
        var ct = TestContext.Current.CancellationToken;
        var host = Host();
        await SetPasswordAsync(host);
        var store = host.Services.GetRequiredService<UiAuthStore>();
        var before = store.PasswordChecks;
        var anonymous = Own(host.CreateAnonymousUiClient());

        var statuses = await Task.WhenAll(Enumerable.Range(0, 60).Select(async i =>
        {
            using var response = await anonymous.PostAsJsonAsync("/api/auth/login", new { password = "wrong password " + i }, ct);
            return response.StatusCode;
        }));

        (store.PasswordChecks - before).Should().BeLessThanOrEqualTo(UiLoginThrottle.FreeFailures);
        statuses.Should().OnlyContain(s => s == HttpStatusCode.Unauthorized || s == HttpStatusCode.TooManyRequests);
        statuses.Count(s => s == HttpStatusCode.Unauthorized).Should().BeLessThanOrEqualTo(UiLoginThrottle.FreeFailures);
    }

    [Fact]
    public async Task A_locked_out_IPv6_host_does_not_lock_out_its_neighbour_on_the_same_network()
    {
        var host = Host();
        await SetPasswordAsync(host);
        var signIn = host.Services.GetRequiredService<UiSignIn>();
        var noisy = UiSignIn.AddressKey(IPAddress.Parse("2001:db8:1:2::10"));
        var neighbour = UiSignIn.AddressKey(IPAddress.Parse("2001:db8:1:2::11"));

        for (var i = 0; i < UiLoginThrottle.FreeFailures; i++)
        {
            (await signIn.LoginAsync(noisy, "wrong" + i)).Verdict.Should().Be(Verdict.Wrong);
        }

        (await signIn.LoginAsync(noisy, KoppelFactory.Password)).Verdict.Should().Be(Verdict.Locked);
        (await signIn.LoginAsync(neighbour, KoppelFactory.Password)).Verdict.Should().Be(Verdict.Ok, "a neighbour in the same /64 is another caller");
    }

    [Fact]
    public async Task A_busy_server_answers_at_once_and_does_not_count_the_try()
    {
        var host = Host();
        await SetPasswordAsync(host);
        var gate = host.Services.GetRequiredService<UiPasswordGate>();
        var held = Enumerable.Range(0, UiPasswordGate.Slots).Select(_ => gate.TryEnter()).ToList();
        var signIn = host.Services.GetRequiredService<UiSignIn>();
        try
        {
            for (var i = 0; i < UiLoginThrottle.FreeFailures + 2; i++)
            {
                (await signIn.LoginAsync("caller", "wrong")).Verdict.Should().Be(Verdict.Busy);
            }
        }
        finally
        {
            held.ForEach(slot => slot?.Dispose());
        }

        (await signIn.LoginAsync("caller", KoppelFactory.Password)).Verdict.Should().Be(Verdict.Ok, "busy answers never counted against the caller");
    }

    [Fact]
    public async Task Setup_login_and_password_change_are_rate_limited_per_address()
    {
        var ct = TestContext.Current.CancellationToken;
        var host = new KoppelFactory(kestrel: true, configuration: new Dictionary<string, string> { ["RateLimits:AuthPerMinute"] = "3" });
        _disposables.Add(host);
        var anonymous = Own(host.CreateAnonymousUiClient());

        for (var i = 0; i < 3; i++)
        {
            (await anonymous.PostAsJsonAsync("/api/auth/login", new { password = "x" }, ct)).StatusCode.Should().Be(HttpStatusCode.Conflict);
        }

        (await anonymous.PostAsJsonAsync("/api/auth/login", new { password = "x" }, ct)).StatusCode.Should().Be(HttpStatusCode.TooManyRequests);
        (await anonymous.PostAsJsonAsync("/api/auth/setup", new { password = "y" }, ct)).StatusCode.Should().Be(HttpStatusCode.TooManyRequests);
        (await anonymous.GetAsync("/api/auth/state", ct)).StatusCode.Should().Be(HttpStatusCode.OK, "the UI polls state, so it is not limited");
    }

    [Fact]
    public async Task A_copied_cookie_is_refused_after_signing_out()
    {
        var ct = TestContext.Current.CancellationToken;
        var host = Host();
        var cookie = await SetPasswordAsync(host);
        var copy = WithCookie(host, cookie);
        (await copy.GetAsync("/api/settings", ct)).StatusCode.Should().Be(HttpStatusCode.OK);

        using var logout = await WithCookie(host, cookie).PostAsync("/api/auth/logout", null, ct);

        logout.StatusCode.Should().Be(HttpStatusCode.NoContent);
        (await copy.GetAsync("/api/settings", ct)).StatusCode.Should().Be(HttpStatusCode.Unauthorized);
        using var login = await Own(host.CreateAnonymousUiClient()).PostAsJsonAsync("/api/auth/login", new { password = KoppelFactory.Password }, ct);
        (await WithCookie(host, CookieOf(login)).GetAsync("/api/settings", ct)).StatusCode.Should().Be(HttpStatusCode.OK, "another session is not affected");
    }

    [Fact]
    public async Task A_copied_cookie_is_still_refused_after_the_host_restarts()
    {
        var ct = TestContext.Current.CancellationToken;
        var host = Host();
        var cookie = await SetPasswordAsync(host);
        using var login = await Own(host.CreateAnonymousUiClient()).PostAsJsonAsync("/api/auth/login", new { password = KoppelFactory.Password }, ct);
        var other = CookieOf(login);
        using var logout = await WithCookie(host, cookie).PostAsync("/api/auth/logout", null, ct);
        logout.StatusCode.Should().Be(HttpStatusCode.NoContent);
        host.Stop();

        var restarted = new KoppelFactory(kestrel: true, restartOf: host);
        _disposables.Add(restarted);

        (await WithCookie(restarted, cookie).GetAsync("/api/settings", ct)).StatusCode.Should().Be(HttpStatusCode.Unauthorized, "the sign-out was written to ui-auth.json");
        (await WithCookie(restarted, other).GetAsync("/api/settings", ct)).StatusCode.Should().Be(HttpStatusCode.OK, "a session that did not sign out is not affected");
    }

    [Fact]
    public async Task The_ui_hub_closes_when_the_cookie_expires()
    {
        var ct = TestContext.Current.CancellationToken;
        var host = new KoppelFactory(kestrel: true, services: s => s.PostConfigure<CookieAuthenticationOptions>(UiSession.Scheme, o => o.ExpireTimeSpan = TimeSpan.FromSeconds(3)));
        _disposables.Add(host);
        var cookie = await SetPasswordAsync(host);
        await using var hub = await ConnectHubAsync(host, cookie, ct);
        var closed = new TaskCompletionSource();
        hub.Closed += _ => { closed.TrySetResult(); return Task.CompletedTask; };

        await closed.Task.WaitAsync(TimeSpan.FromSeconds(20), ct);
    }

    [Fact]
    public async Task No_setup_code_file_is_written_and_the_password_file_is_encrypted()
    {
        var host = Host();

        await SetPasswordAsync(host);

        File.Exists(host.ConfigFile("setup-code.txt")).Should().BeFalse();
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
        var ui = Own(host.CreateUiClient());
        var anonymous = Own(host.CreateAnonymousUiClient());

        for (var i = 0; i < UiLoginThrottle.FreeFailures; i++)
        {
            (await anonymous.PostAsJsonAsync("/api/auth/login", new { password = "wrong" + i }, ct)).StatusCode.Should().Be(HttpStatusCode.Unauthorized);
        }

        using var locked = await anonymous.PostAsJsonAsync("/api/auth/login", new { password = KoppelFactory.Password }, ct);
        locked.StatusCode.Should().Be(HttpStatusCode.TooManyRequests, "the right password does not help during a lock");
        locked.Headers.RetryAfter!.Delta.Should().BeGreaterThan(TimeSpan.FromSeconds(30));
        (await ErrorOf(locked, ct)).Should().Contain("Wait");

        var activity = await ui.GetFromJsonAsync<JsonElement>("/api/activity?max=50", ct);
        activity.EnumerateArray().Count(e => e.GetProperty("reason").GetString() == "signInFailed" && e.GetProperty("outcome").GetString() == "failed")
            .Should().Be(1, "one entry when the lock engages, not one per wrong try");
    }

    [Fact]
    public async Task The_session_cookie_is_http_only_strict_and_named_koppel_ui()
    {
        var ct = TestContext.Current.CancellationToken;
        var host = Host();
        var anonymous = Own(host.CreateAnonymousUiClient());

        using var response = await anonymous.PostAsJsonAsync("/api/auth/setup", new { password = KoppelFactory.Password }, ct);

        var cookie = response.Headers.GetValues("Set-Cookie").Single();
        cookie.Should().StartWith("koppel.ui=");
        cookie.ToLowerInvariant().Should().Contain("httponly").And.Contain("samesite=strict").And.Contain("secure");
        cookie.ToLowerInvariant().Should().NotContain("expires=", "a session cookie; the ticket carries the 12 hours");

        var options = host.Services.GetRequiredService<IOptionsMonitor<CookieAuthenticationOptions>>().Get(UiSession.Scheme);
        options.Cookie.HttpOnly.Should().BeTrue();
        options.Cookie.SameSite.Should().Be(SameSiteMode.Strict);
        options.Cookie.SecurePolicy.Should().Be(CookieSecurePolicy.Always, "the UI is HTTPS only");
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
    public async Task Resetting_the_password_clears_it_ends_sessions_and_opens_setup_again()
    {
        var ct = TestContext.Current.CancellationToken;
        var clock = new ManualTimeProvider();
        var host = Host(clock);
        var cookie = await SetPasswordAsync(host);
        var paths = host.Services.GetRequiredService<KoppelPaths>();
        var store = host.Services.GetRequiredService<UiAuthStore>();
        var output = new StringWriter();
        // Long after the window Koppel opened at start.
        clock.Advance(TimeSpan.FromHours(1));

        var exit = await UiPasswordReset.RunAsync(paths, host.Services.GetRequiredService<SecretBox>(), UiSetupWindow.DefaultLength, output);

        exit.Should().Be(0);
        output.ToString().Should().Contain("10 minutes").And.NotContain("setup code");

        // The running host sees the file change.
        var deadline = DateTime.UtcNow.AddSeconds(15);
        while (store.HasPassword && DateTime.UtcNow < deadline)
        {
            await Task.Delay(100, ct);
        }

        store.HasPassword.Should().BeFalse();
        store.SetupOpen.Should().BeTrue("a reset opens setup again at once");
        store.SetupUntil.Should().Be(clock.GetUtcNow() + UiSetupWindow.DefaultLength);
        (await WithCookie(host, cookie).GetAsync("/api/settings", ct)).StatusCode.Should().Be(HttpStatusCode.Unauthorized);
        using var login = await Own(host.CreateAnonymousUiClient()).PostAsJsonAsync("/api/auth/login", new { password = KoppelFactory.Password }, ct);
        login.StatusCode.Should().Be(HttpStatusCode.Conflict);

        await SetPasswordAsync(host);
        store.SetupOpen.Should().BeFalse();
    }

    [Fact]
    public async Task A_reset_while_stopped_opens_setup_on_the_next_start()
    {
        var host = Host();
        await SetPasswordAsync(host);
        var paths = host.Services.GetRequiredService<KoppelPaths>();
        host.Stop();
        using (var box = Koppel.Security.SecretBoxFactory.Create(paths, Microsoft.Extensions.Logging.Abstractions.NullLogger.Instance))
        {
            (await UiPasswordReset.RunAsync(paths, box, UiSetupWindow.DefaultLength, new StringWriter())).Should().Be(0);
        }

        var clock = new ManualTimeProvider();
        var restarted = new KoppelFactory(kestrel: true, restartOf: host, services: s => s.AddSingleton<TimeProvider>(clock));
        _disposables.Add(restarted);

        var store = restarted.Services.GetRequiredService<UiAuthStore>();
        store.HasPassword.Should().BeFalse();
        store.SetupUntil.Should().Be(clock.GetUtcNow() + UiSetupWindow.DefaultLength);
        await SetPasswordAsync(restarted);
    }

    [Fact]
    public async Task The_log_says_where_to_set_the_first_password_and_until_when_and_an_old_code_file_goes()
    {
        var directory = Path.Combine(Path.GetTempPath(), "koppel-tests", Guid.NewGuid().ToString("N"));
        try
        {
            var paths = new KoppelPaths(Path.Combine(directory, "config"), Path.Combine(directory, "keys"));
            paths.Prepare();
            await File.WriteAllTextAsync(paths.OldSetupCodeFile, "ABCD-2345", TestContext.Current.CancellationToken);
            using var box = new SecretBox(paths.KeysDirectory, "Koppel.Secrets.v1");
            var logger = new CapturingLogger();
            var clock = new ManualTimeProvider();
            var window = new UiSetupWindow(UiSetupWindow.DefaultLength, "https://bench-pc:5240");
            using var store = new UiAuthStore(paths, box, window, clock, logger);

            await store.StartAsync(TestContext.Current.CancellationToken);

            File.Exists(paths.OldSetupCodeFile).Should().BeFalse();
            var until = TimeZoneInfo.ConvertTime(clock.GetUtcNow() + window.Length, clock.LocalTimeZone);
            logger.Messages.Should().ContainSingle(m => m.Contains("waiting for its first password"))
                .Which.Should().Contain("https://bench-pc:5240").And.Contain(until.ToString("yyyy-MM-dd HH:mm:ss", System.Globalization.CultureInfo.InvariantCulture));
            store.SetupOpen.Should().BeTrue();

            await store.StopAsync(TestContext.Current.CancellationToken);
        }
        finally
        {
            Directory.Delete(directory, recursive: true);
        }
    }

    private sealed class CapturingLogger : Microsoft.Extensions.Logging.ILogger<UiAuthStore>
    {
        public List<string> Messages { get; } = [];

        public IDisposable? BeginScope<TState>(TState state) where TState : notnull => null;

        public bool IsEnabled(Microsoft.Extensions.Logging.LogLevel logLevel) => true;

        public void Log<TState>(Microsoft.Extensions.Logging.LogLevel logLevel, Microsoft.Extensions.Logging.EventId eventId, TState state, Exception? exception, Func<TState, Exception?, string> formatter) =>
            Messages.Add(formatter(state, exception));
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

            (await UiPasswordReset.RunAsync(paths, box, UiSetupWindow.DefaultLength, new StringWriter())).Should().Be(0);
            File.Exists(paths.UiAuthFile).Should().BeTrue();

            await File.WriteAllTextAsync(paths.UiAuthFile, "not encrypted at all", TestContext.Current.CancellationToken);
            (await UiPasswordReset.RunAsync(paths, box, UiSetupWindow.DefaultLength, new StringWriter())).Should().Be(0);
            using var store = new UiAuthStore(paths, box, new UiSetupWindow(UiSetupWindow.DefaultLength, "https://localhost:5240"), TimeProvider.System, Microsoft.Extensions.Logging.Abstractions.NullLogger<UiAuthStore>.Instance);
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
            .WithUrl($"https://localhost:{host.HttpPort}/api/hubs/ui", options =>
            {
                host.TrustCertificate(options);
                options.Headers["Cookie"] = cookie;
            })
            .Build();
        await hub.StartAsync(ct);
        return hub;
    }
}
