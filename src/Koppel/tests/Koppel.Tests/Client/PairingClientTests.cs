using System.Net;
using System.Text;
using System.Text.Json;
using Koppel.Client;
using Koppel.Core.Configuration;
using Koppel.Core.Security;
using Koppel.Server;
using Microsoft.Extensions.Logging.Abstractions;

namespace Koppel.Tests.Client;

/// <summary>
/// The client side of pairing against a fake server and a manual clock: who may set the status, what
/// an approval must carry, and when credentials are saved straight away or wait for the person.
/// </summary>
public sealed class PairingClientTests : IAsyncLifetime
{
    private const string FingerprintA = "AA11AA11AA11AA11AA11AA11AA11AA11AA11AA11AA11AA11AA11AA11AA11AA11";
    private const string FingerprintB = "BB22BB22BB22BB22BB22BB22BB22BB22BB22BB22BB22BB22BB22BB22BB22BB22";
    private const string UrlA = "https://server-a.test:5241";
    private const string UrlB = "https://server-b.test:5241";

    private readonly string _directory = Path.Combine(Path.GetTempPath(), "koppel-tests", Guid.NewGuid().ToString("N"));
    private readonly ManualTimeProvider _time = new();
    private readonly Dictionary<string, FakeServer> _servers = [];
    private readonly List<(Func<PairingStatus, bool> Match, TaskCompletionSource<PairingStatus> Done)> _waiters = [];
    private readonly List<PairingStatus> _statuses = [];
    private readonly Lock _sync = new();
    private SecretBox _box = null!;
    private ConnectionStore _connection = null!;
    private PairingClient _client = null!;

    public async ValueTask InitializeAsync()
    {
        _box = new SecretBox(Path.Combine(_directory, "keys"), "test");
        _connection = new ConnectionStore(new KoppelPaths(Path.Combine(_directory, "config"), Path.Combine(_directory, "keys")), _box, NullLogger<ConnectionStore>.Instance);
        await _connection.StartAsync(TestContext.Current.CancellationToken);
        _client = new PairingClient(_connection, _time, NullLogger<PairingClient>.Instance) { HandlerFactory = fingerprint => _servers[fingerprint] };
        _client.Changed += status =>
        {
            lock (_sync)
            {
                _statuses.Add(status);
                foreach (var waiter in _waiters.Where(w => w.Match(status)).ToList())
                {
                    _waiters.Remove(waiter);
                    waiter.Done.TrySetResult(status);
                }
            }

            return Task.CompletedTask;
        };
    }

    public ValueTask DisposeAsync()
    {
        _client.Dispose();
        _connection.Dispose();
        _box.Dispose();
        if (Directory.Exists(_directory))
        {
            Directory.Delete(_directory, recursive: true);
        }

        return ValueTask.CompletedTask;
    }

    [Fact]
    public async Task Approval_for_a_client_with_no_connection_is_saved_as_given()
    {
        _servers[FingerprintA] = FakeServer.Approving("client-1", "new-secret");

        await PairAsync(UrlA, FingerprintA, "paired");

        _connection.Current.ClientId.Should().Be("client-1");
        _connection.Current.ClientSecret.Should().Be("new-secret");
        _connection.Current.ServerUrl.Should().Be(UrlA);
        ConnectionUpdate.SameFingerprint(_connection.Current.ServerFingerprint, FingerprintA).Should().BeTrue();
    }

    [Theory]
    [InlineData("client-1", "")]
    [InlineData("client-1", null)]
    [InlineData("", "secret")]
    [InlineData(null, null)]
    public async Task Approval_without_credentials_fails_and_saves_nothing(string? clientId, string? secret)
    {
        await _connection.UpdateAsync(_ => new ClientConnection(UrlA, FingerprintA, "old-id", "old-secret"), TestContext.Current.CancellationToken);
        _servers[FingerprintA] = FakeServer.Approving(clientId, secret);

        var status = await PairAsync(UrlA, FingerprintA, "failed");

        status.Message.Should().Contain("credentials");
        _connection.Current.Should().Be(new ClientConnection(UrlA, FingerprintA, "old-id", "old-secret"), "the stored secret is never merged in or dropped");
    }

    [Fact]
    public async Task Approval_for_another_server_waits_until_the_person_confirms()
    {
        var ct = TestContext.Current.CancellationToken;
        await _connection.UpdateAsync(_ => new ClientConnection(UrlB, FingerprintB, "old-id", "old-secret"), ct);
        _servers[FingerprintA] = FakeServer.Approving("client-1", "new-secret");

        var held = await PairAsync(UrlA, FingerprintA, "confirm");

        held.ServerName.Should().Be("server-a.test");
        ConnectionUpdate.SameFingerprint(held.ServerFingerprint, FingerprintA).Should().BeTrue();
        _connection.Current.Should().Be(new ClientConnection(UrlB, FingerprintB, "old-id", "old-secret"), "nothing is saved yet");

        (await _client.ConfirmAsync(ct)).Should().BeTrue();

        _client.Current.State.Should().Be("paired");
        _connection.Current.ServerUrl.Should().Be(UrlA);
        _connection.Current.ClientId.Should().Be("client-1");
        _connection.Current.ClientSecret.Should().Be("new-secret", "the old server's secret is not carried over");
        (await _client.ConfirmAsync(ct)).Should().BeFalse("there is nothing left to confirm");
    }

    [Fact]
    public async Task Discarding_a_held_approval_keeps_the_connection_and_drops_the_credentials()
    {
        var ct = TestContext.Current.CancellationToken;
        var old = new ClientConnection(UrlB, FingerprintB, "old-id", "old-secret");
        await _connection.UpdateAsync(_ => old, ct);
        _servers[FingerprintA] = FakeServer.Approving("client-1", "new-secret");
        await PairAsync(UrlA, FingerprintA, "confirm");

        await _client.CancelAsync();

        _client.Current.State.Should().Be("idle");
        (await _client.ConfirmAsync(ct)).Should().BeFalse();
        _connection.Current.Should().Be(old);
    }

    [Fact]
    public async Task Approval_for_the_server_already_connected_replaces_the_credentials_without_asking()
    {
        await _connection.UpdateAsync(_ => new ClientConnection(UrlA, FingerprintA, "old-id", "old-secret"), TestContext.Current.CancellationToken);
        _servers[FingerprintA] = FakeServer.Approving("client-2", "fresh-secret");

        await PairAsync(UrlA, FingerprintA, "paired");

        _connection.Current.ClientId.Should().Be("client-2");
        _connection.Current.ClientSecret.Should().Be("fresh-secret");
    }

    [Fact]
    public async Task A_run_that_was_replaced_cannot_overwrite_the_status_of_the_newer_one()
    {
        var release = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        _servers[FingerprintA] = new FakeServer
        {
            Open = async _ =>
            {
                await release.Task;
                return new HttpResponseMessage(HttpStatusCode.TooManyRequests);
            },
        };
        _servers[FingerprintB] = FakeServer.Waiting();
        var older = _client.Begin(UrlA, FingerprintA, "old");
        await WaitForAsync(s => s.State == "requesting" && s.ServerName == "old");

        var newer = _client.Begin(UrlB, FingerprintB, "new");
        await WaitForAsync(s => s.State == "waiting" && s.ServerName == "new");
        release.SetResult();
        await older;

        _client.Current.State.Should().Be("waiting");
        _client.Current.ServerName.Should().Be("new");
        lock (_sync)
        {
            _statuses.Where(s => s.ServerName == "old").Select(s => s.State).Should().Equal("requesting");
        }

        await _client.CancelAsync();
        await newer;
    }

    [Fact]
    public async Task An_approval_that_arrives_for_a_replaced_run_is_not_saved()
    {
        var release = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var polling = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        _servers[FingerprintA] = new FakeServer
        {
            Poll = async _ =>
            {
                polling.SetResult();
                await release.Task;
                return FakeServer.Json(new PairingPoll("approved", "client-1", "new-secret"));
            },
        };
        _servers[FingerprintB] = FakeServer.Waiting();
        var older = _client.Begin(UrlA, FingerprintA, "old");
        await WaitForAsync(s => s.State == "waiting" && s.ServerName == "old");
        await _time.WhenTimersAsync(2);
        _time.Advance(TimeSpan.FromSeconds(2));
        await polling.Task;

        var newer = _client.Begin(UrlB, FingerprintB, "new");
        await WaitForAsync(s => s.State == "waiting" && s.ServerName == "new");
        release.SetResult();
        await older;

        _connection.Current.ClientId.Should().BeNull("the run that asked is no longer the one the person is looking at");
        _client.Current.ServerName.Should().Be("new");
        await _client.CancelAsync();
        await newer;
    }

    [Fact]
    public async Task A_reply_the_client_cannot_read_ends_as_a_readable_failure()
    {
        _servers[FingerprintA] = new FakeServer { Open = _ => Task.FromResult(new HttpResponseMessage(HttpStatusCode.OK) { Content = new StringContent("<html>proxy</html>", Encoding.UTF8, "text/html") }) };

        var status = await PairAsync(UrlA, FingerprintA, "failed");

        status.Message.Should().StartWith("Could not pair with server-a.test");
    }

    [Fact]
    public async Task An_open_reply_without_a_poll_token_is_a_failure()
    {
        _servers[FingerprintA] = new FakeServer { Open = _ => Task.FromResult(FakeServer.Json(new PairOpened("req-1", "server-nonce", _time.GetUtcNow().AddMinutes(5), ""))) };

        var status = await PairAsync(UrlA, FingerprintA, "failed");

        status.Message.Should().Contain("not what Koppel expects");
    }

    [Fact]
    public async Task A_server_error_while_polling_is_a_failure_not_a_silent_stop()
    {
        _servers[FingerprintA] = new FakeServer
        {
            Poll = _ => Task.FromResult(new HttpResponseMessage(HttpStatusCode.InternalServerError)),
        };

        var status = await PairAsync(UrlA, FingerprintA, "failed");

        status.Message.Should().Contain("500");
    }

    [Fact]
    public async Task Polls_and_the_withdraw_present_the_poll_token()
    {
        var server = FakeServer.Waiting();
        _servers[FingerprintA] = server;
        var run = _client.Begin(UrlA, FingerprintA, null);
        await WaitForAsync(s => s.State == "waiting");
        await _time.WhenTimersAsync(2);
        _time.Advance(TimeSpan.FromSeconds(2));
        await server.PolledAsync;

        await _client.CancelAsync();
        await run;

        server.Calls.Should().Contain(c => c.Method == "GET" && c.Token == "tok-1");
        server.Calls.Should().Contain(c => c.Method == "DELETE" && c.Token == "tok-1");
    }

    [Fact]
    public async Task Paired_is_cleared_once_the_stored_connection_is_another_one()
    {
        var ct = TestContext.Current.CancellationToken;
        _servers[FingerprintA] = FakeServer.Approving("client-1", "new-secret");
        await PairAsync(UrlA, FingerprintA, "paired");
        var cleared = WaitForAsync(s => s.State == "idle");

        await _connection.UpdateAsync(c => c with { ServerUrl = UrlB }, ct);

        await cleared;
        _client.Current.State.Should().Be("idle");
    }

    /// <summary>Starts a run and drives the clock until it reaches <paramref name="state"/>.</summary>
    private async Task<PairingStatus> PairAsync(string url, string fingerprint, string state)
    {
        var done = WaitForAsync(s => s.State == state);
        var run = _client.Begin(url, fingerprint, null);
        while (!done.IsCompleted && !run.IsCompleted)
        {
            await Task.WhenAny(done, run, _time.WhenTimersAsync(2));
            if (!done.IsCompleted && !run.IsCompleted)
            {
                _time.Advance(TimeSpan.FromSeconds(2));
            }
        }

        return await done.WaitAsync(TimeSpan.FromSeconds(10), TestContext.Current.CancellationToken);
    }

    private Task<PairingStatus> WaitForAsync(Func<PairingStatus, bool> match)
    {
        lock (_sync)
        {
            if (_statuses.LastOrDefault(match) is { } seen)
            {
                return Task.FromResult(seen);
            }

            var done = new TaskCompletionSource<PairingStatus>(TaskCreationOptions.RunContinuationsAsynchronously);
            _waiters.Add((match, done));
            return done.Task.WaitAsync(TimeSpan.FromSeconds(10), TestContext.Current.CancellationToken);
        }
    }

    /// <summary>A server that answers the three pairing calls, each replaceable.</summary>
    private sealed class FakeServer : HttpMessageHandler
    {
        private readonly TaskCompletionSource _polled = new(TaskCreationOptions.RunContinuationsAsynchronously);

        public Func<HttpRequestMessage, Task<HttpResponseMessage>>? Open { get; init; }

        public Func<HttpRequestMessage, Task<HttpResponseMessage>>? Poll { get; init; }

        public List<(string Method, string Token)> Calls { get; } = [];

        public Task PolledAsync => _polled.Task;

        public static FakeServer Waiting() => new() { Poll = _ => Task.FromResult(Json(new PairingPoll("waiting"))) };

        public static FakeServer Approving(string? clientId, string? secret) =>
            new() { Poll = _ => Task.FromResult(Json(new PairingPoll("approved", clientId, secret))) };

        public static HttpResponseMessage Json<T>(T value) =>
            new(HttpStatusCode.OK) { Content = new StringContent(JsonSerializer.Serialize(value, KoppelJson.Files), Encoding.UTF8, "application/json") };

        protected override async Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken)
        {
            var token = request.Headers.TryGetValues(PairingEndpoints.TokenHeader, out var values) ? values.Single() : "";
            lock (Calls)
            {
                Calls.Add((request.Method.Method, token));
            }

            var path = request.RequestUri!.AbsolutePath;
            if (request.Method == HttpMethod.Post && path == "/api/pair")
            {
                return Open is null
                    ? Json(new PairOpened("req-1", "server-nonce", DateTimeOffset.UtcNow.AddMinutes(5), "tok-1"))
                    : await Open(request);
            }

            if (request.Method == HttpMethod.Post && path.EndsWith("/reveal", StringComparison.Ordinal))
            {
                return new HttpResponseMessage(HttpStatusCode.NoContent);
            }

            if (request.Method == HttpMethod.Get)
            {
                _polled.TrySetResult();
                return Poll is null ? Json(new PairingPoll("waiting")) : await Poll(request);
            }

            return new HttpResponseMessage(HttpStatusCode.NoContent);
        }
    }
}
