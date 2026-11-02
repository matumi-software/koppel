using System.Net;
using System.Text;
using Koppel.Core.Auth;

namespace Koppel.Tests.Server;

/// <summary>What the token client does with answers a Koppel server would never send, such as an error page from a proxy.</summary>
public sealed class TokenClientTests
{
    private static readonly Uri Server = new("https://lab-rack-3:5241");
    private readonly ManualTimeProvider _time = new();

    [Fact]
    public async Task A_success_with_a_token_is_returned_and_reused_until_it_nearly_expires()
    {
        var calls = 0;
        using var client = ClientFor(_ =>
        {
            calls++;
            return Reply(HttpStatusCode.OK, "application/json", """{"access_token":"tok","expires_in":300}""");
        });
        var ct = TestContext.Current.CancellationToken;

        (await client.GetAccessTokenAsync(Server, "id", "secret", ct)).Should().Be("tok");
        (await client.GetAccessTokenAsync(Server, "id", "secret", ct)).Should().Be("tok");
        calls.Should().Be(1);

        _time.Advance(TimeSpan.FromSeconds(250));
        await client.GetAccessTokenAsync(Server, "id", "secret", ct);
        calls.Should().Be(2);
    }

    [Fact]
    public async Task An_error_status_with_an_html_body_is_a_readable_token_error()
    {
        using var client = ClientFor(_ => Reply(HttpStatusCode.BadGateway, "text/html", "<html>Bad gateway</html>"));

        var error = (await client.Invoking(c => c.GetAccessTokenAsync(Server, "id", "secret", TestContext.Current.CancellationToken))
            .Should().ThrowAsync<TokenRequestException>()).Which;

        error.Error.Should().Be("http_502");
        error.Message.Should().Contain("502");
    }

    [Fact]
    public async Task An_oauth_error_keeps_its_code_and_description()
    {
        using var client = ClientFor(_ => Reply(HttpStatusCode.Unauthorized, "application/json", """{"error":"invalid_client","error_description":"The client credentials are invalid."}"""));

        var error = (await client.Invoking(c => c.GetAccessTokenAsync(Server, "id", "secret", TestContext.Current.CancellationToken))
            .Should().ThrowAsync<TokenRequestException>()).Which;

        error.Error.Should().Be("invalid_client");
        error.Message.Should().Contain("credentials are invalid");
    }

    [Theory]
    [InlineData("text/html", "<html>login</html>")]
    [InlineData("application/json", "")]
    [InlineData("application/json", "{not json")]
    [InlineData("application/json", "{}")]
    public async Task A_success_that_is_not_a_token_response_is_a_token_error(string contentType, string body)
    {
        using var client = ClientFor(_ => Reply(HttpStatusCode.OK, contentType, body));

        var error = (await client.Invoking(c => c.GetAccessTokenAsync(Server, "id", "secret", TestContext.Current.CancellationToken))
            .Should().ThrowAsync<TokenRequestException>()).Which;

        error.Error.Should().Be("invalid_response");
    }

    private KoppelTokenClient ClientFor(Func<HttpRequestMessage, HttpResponseMessage> respond) =>
        new(new HttpClient(new Fake(respond)), _time);

    private static HttpResponseMessage Reply(HttpStatusCode status, string contentType, string body) =>
        new(status) { Content = new StringContent(body, Encoding.UTF8, contentType) };

    private sealed class Fake(Func<HttpRequestMessage, HttpResponseMessage> respond) : HttpMessageHandler
    {
        protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken) =>
            Task.FromResult(respond(request));
    }
}
