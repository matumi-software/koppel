using System.Net;
using Koppel.Web;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Hosting;
using Microsoft.Extensions.DependencyInjection;

namespace Koppel.Tests.Web;

public sealed class UiAccessTests
{
    [Fact]
    public void Resolve_is_lan_unless_told_otherwise_and_local_in_development()
    {
        UiAccess.Resolve(null, development: false).Should().Be(UiReach.Lan);
        UiAccess.Resolve("", development: true).Should().Be(UiReach.Local);
        UiAccess.Resolve(" ANY ", development: true).Should().Be(UiReach.Any);
    }

    [Fact]
    public void Resolve_rejects_an_unknown_value_and_names_the_setting()
    {
        var resolve = () => UiAccess.Resolve("everyone", development: false);

        resolve.Should().Throw<InvalidOperationException>().Which.Message.Should().Contain("Ui:AllowFrom");
    }

    [Theory]
    [InlineData("127.0.0.1", true)]
    [InlineData("::1", true)]
    [InlineData("::ffff:127.0.0.1", true)]
    [InlineData("192.168.1.5", false)]
    [InlineData("fe80::1", false)]
    public void Local_allows_this_machine_only(string address, bool allowed) =>
        new UiAccess(UiReach.Local, NoNetworks).Allows(IPAddress.Parse(address)).Should().Be(allowed);

    [Theory]
    [InlineData("127.0.0.1", true)]
    [InlineData("10.1.2.3", true)]
    [InlineData("172.20.0.1", true)]
    [InlineData("::ffff:192.168.1.5", true)]
    [InlineData("169.254.10.1", true)]
    [InlineData("fe80::1", true)]
    [InlineData("fd12:3456::1", true)]
    [InlineData("8.8.8.8", false)]
    [InlineData("172.32.0.1", false)]
    [InlineData("100.64.0.1", false)]
    [InlineData("2001:db8:9::1", false)]
    public void Lan_allows_private_and_link_local_addresses(string address, bool allowed) =>
        new UiAccess(UiReach.Lan, NoNetworks).Allows(IPAddress.Parse(address)).Should().Be(allowed);

    [Fact]
    public void Lan_allows_a_network_this_machine_has_an_address_on()
    {
        var access = new UiAccess(UiReach.Lan, () => [(IPAddress.Parse("2001:db8:1:2::10"), 64), (IPAddress.Parse("203.0.113.7"), 24)]);

        access.Allows(IPAddress.Parse("2001:db8:1:2::99")).Should().BeTrue();
        access.Allows(IPAddress.Parse("203.0.113.200")).Should().BeTrue();
        access.Allows(IPAddress.Parse("2001:db8:1:3::1")).Should().BeFalse();
        access.Allows(IPAddress.Parse("203.0.114.1")).Should().BeFalse();
    }

    [Fact]
    public void Any_allows_everyone_and_no_address_is_always_allowed()
    {
        new UiAccess(UiReach.Any, NoNetworks).Allows(IPAddress.Parse("8.8.8.8")).Should().BeTrue();
        new UiAccess(UiReach.Local, NoNetworks).Allows(null).Should().BeTrue();
    }

    [Fact]
    public async Task A_ui_request_from_a_refused_address_gets_403_and_the_remote_routes_do_not_care()
    {
        var ct = TestContext.Current.CancellationToken;
        using var host = new KoppelFactory(
            services: s => s.AddTransient<IStartupFilter>(_ => new FromAddress(IPAddress.Parse("203.0.113.5"))),
            configuration: new Dictionary<string, string> { ["Ui:AllowFrom"] = "lan" });
        using var ui = host.CreateAnonymousUiClient();
        using var https = host.CreateHttpsClient();
        var form = new Dictionary<string, string> { ["grant_type"] = "client_credentials", ["client_id"] = "x", ["client_secret"] = "y" };

        (await ui.GetAsync("/api/auth/state", ct)).StatusCode.Should().Be(HttpStatusCode.Forbidden);
        (await ui.GetAsync("/", ct)).StatusCode.Should().Be(HttpStatusCode.Forbidden);
        (await https.PostAsync("/connect/token", new FormUrlEncodedContent(form), ct)).StatusCode.Should().Be(HttpStatusCode.Unauthorized);
    }

    private static IReadOnlyList<(IPAddress Network, int PrefixLength)> NoNetworks() => [];

    /// <summary>Makes every request on a test server come from one address.</summary>
    private sealed class FromAddress(IPAddress address) : IStartupFilter
    {
        public Action<IApplicationBuilder> Configure(Action<IApplicationBuilder> next) => app =>
        {
            app.Use((context, nextMiddleware) =>
            {
                context.Connection.RemoteIpAddress = address;
                return nextMiddleware(context);
            });
            next(app);
        };
    }
}