using System.Net;
using Koppel.Client;
using Koppel.Core.Security;
using Microsoft.Extensions.Logging.Abstractions;
using Skakel.Client;

namespace Koppel.Tests.Client;

/// <summary>
/// The client side of pairing against fake requests: who may set the status, what an approval must carry,
/// how Skakel's failures read, and when credentials are saved straight away or wait for the person. The
/// protocol underneath is Skakel's, with tests of its own; <c>Server/PairingTests</c> runs it end to end.
/// </summary>
public sealed class PairingClientTests : IAsyncLifetime
{
    private const string FingerprintA = "AA11AA11AA11AA11AA11AA11AA11AA11AA11AA11AA11AA11AA11AA11AA11AA11";
    private const string FingerprintB = "BB22BB22BB22BB22BB22BB22BB22BB22BB22BB22BB22BB22BB22BB22BB22BB22";
    private const string UrlA = "https://server-a.test:5241";
    private const string UrlB = "https://server-b.test:5241";

    private readonly string _directory = Path.Combine(Path.GetTempPath(), "koppel-tests", Guid.NewGuid().ToString("N"));
    private readonly Dictionary<string, Func<FakeRequest>> _servers = [];
    private readonly List<(Func<PairingStatus, bool> Match, TaskCompletionSource<PairingStatus> Done)> _waiters = [];
    private readonly List<PairingStatus> _statuses = [];
    private readonly List<FakeRequest> _opened = [];
    private readonly Lock _sync = new();
    private SecretBox _box = null!;
    private ConnectionStore _connection = null!;
    private PairingClient _client = null!;

    public async ValueTask InitializeAsync()
    {
        _box = new SecretBox(Path.Combine(_directory, "keys"), "test");
        _connection = new ConnectionStore(new KoppelPaths(Path.Combine(_directory, "config"), Path.Combine(_directory, "keys")), _box, NullLogger<ConnectionStore>.Instance);
        await _connection.StartAsync(TestContext.Current.CancellationToken);
        _client = new PairingClient(_connection, TimeProvider.System, NullLoggerFactory.Instance) { Open = OpenAsync };
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
        _servers[FingerprintA] = () => FakeRequest.Approving("client-1", "new-secret");

        await PairAsync(UrlA, FingerprintA, "paired");

        _connection.Current.ClientId.Should().Be("client-1");
        _connection.Current.ClientSecret.Should().Be("new-secret");
        _connection.Current.ServerUrl.Should().Be(UrlA);
        ConnectionUpdate.SameFingerprint(_connection.Current.ServerFingerprint, FingerprintA).Should().BeTrue();
    }

    [Theory]
    [InlineData("client-1", "")]
    [InlineData("", "secret")]
    [InlineData(" ", "secret")]
    public async Task Approval_without_credentials_fails_and_saves_nothing(string clientId, string secret)
    {
        await _connection.UpdateAsync(_ => new ClientConnection(UrlA, FingerprintA, "old-id", "old-secret"), TestContext.Current.CancellationToken);
        _servers[FingerprintA] = () => FakeRequest.Approving(clientId, secret);

        var status = await PairAsync(UrlA, FingerprintA, "failed");

        status.Message.Should().Contain("credentials");
        _connection.Current.Should().Be(new ClientConnection(UrlA, FingerprintA, "old-id", "old-secret"), "the stored secret is never merged in or dropped");
    }

    [Fact]
    public async Task Approval_for_another_server_waits_until_the_person_confirms()
    {
        var ct = TestContext.Current.CancellationToken;
        await _connection.UpdateAsync(_ => new ClientConnection(UrlB, FingerprintB, "old-id", "old-secret"), ct);
        _servers[FingerprintA] = () => FakeRequest.Approving("client-1", "new-secret");

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
        _servers[FingerprintA] = () => FakeRequest.Approving("client-1", "new-secret");
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
        _servers[FingerprintA] = () => FakeRequest.Approving("client-2", "fresh-secret");

        await PairAsync(UrlA, FingerprintA, "paired");

        _connection.Current.ClientId.Should().Be("client-2");
        _connection.Current.ClientSecret.Should().Be("fresh-secret");
    }

    [Fact]
    public async Task A_run_that_was_replaced_cannot_overwrite_the_status_of_the_newer_one()
    {
        var release = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        _servers[FingerprintA] = () => new FakeRequest { Opening = release.Task, Fails = new SkakelException(SkakelFailure.Of(SkakelFailureKind.TooManyRequests)) };
        _servers[FingerprintB] = () => new FakeRequest();
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
        _servers[FingerprintA] = () => new FakeRequest { Approval = release.Task.ContinueWith(_ => new SkakelCredentials("client-1", "new-secret"), TaskScheduler.Default), IgnoresCancel = true };
        _servers[FingerprintB] = () => new FakeRequest();
        var older = _client.Begin(UrlA, FingerprintA, "old");
        await WaitForAsync(s => s.State == "waiting" && s.ServerName == "old");

        var newer = _client.Begin(UrlB, FingerprintB, "new");
        await WaitForAsync(s => s.State == "waiting" && s.ServerName == "new");
        release.SetResult();
        await older;

        _connection.Current.ClientId.Should().BeNull("the run that asked is no longer the one the person is looking at");
        _client.Current.ServerName.Should().Be("new");
        await _client.CancelAsync();
        await newer;
    }

    [Theory]
    [InlineData(SkakelFailureKind.PairingDenied, "denied", "server-a.test refused the request.")]
    [InlineData(SkakelFailureKind.PairingExpired, "expired", "Nobody typed the code on the server in time. Start again.")]
    [InlineData(SkakelFailureKind.TooManyRequests, "failed", "server-a.test has too many pairing requests open. Try again in a few minutes.")]
    [InlineData(SkakelFailureKind.CertificateMismatch, "failed", "Could not pair with server-a.test: The server's certificate does not match the one this app trusts. If the server has a new certificate, pair with it again.")]
    public async Task Skakels_failures_end_the_run_in_fixed_words(SkakelFailureKind kind, string state, string message)
    {
        _servers[FingerprintA] = () => new FakeRequest { Approval = Task.FromException<SkakelCredentials>(new SkakelException(SkakelFailure.Of(kind))) };

        var status = await PairAsync(UrlA, FingerprintA, state);

        status.Message.Should().Be(message);
        status.Code.Should().BeNull();
    }

    [Fact]
    public async Task A_machine_that_is_a_client_only_is_named_as_one()
    {
        var notFound = new HttpRequestException("Not found", null, HttpStatusCode.NotFound);
        _servers[FingerprintA] = () => new FakeRequest { Fails = new SkakelException(SkakelFailure.Of(SkakelFailureKind.UnexpectedAnswer), notFound) };

        var status = await PairAsync(UrlA, FingerprintA, "failed");

        status.Message.Should().Be("server-a.test does not take pairing requests. It may be set up as a client only.");
    }

    [Fact]
    public async Task Cancelling_disposes_the_request_so_it_is_withdrawn()
    {
        _servers[FingerprintA] = () => new FakeRequest();
        var run = _client.Begin(UrlA, FingerprintA, null);
        await WaitForAsync(s => s.State == "waiting");

        await _client.CancelAsync();
        await run;

        lock (_sync)
        {
            _opened.Should().ContainSingle().Which.Disposed.Should().BeTrue();
        }

        _client.Current.State.Should().Be("idle");
    }

    [Fact]
    public async Task Paired_is_cleared_once_the_stored_connection_is_another_one()
    {
        var ct = TestContext.Current.CancellationToken;
        _servers[FingerprintA] = () => FakeRequest.Approving("client-1", "new-secret");
        await PairAsync(UrlA, FingerprintA, "paired");
        var cleared = WaitForAsync(s => s.State == "idle");

        await _connection.UpdateAsync(c => c with { ServerUrl = UrlB }, ct);

        await cleared;
        _client.Current.State.Should().Be("idle");
    }

    private async Task<IPairingRequest> OpenAsync(Uri server, string fingerprint, CancellationToken cancellationToken)
    {
        var request = _servers[fingerprint]();
        lock (_sync)
        {
            _opened.Add(request);
        }

        await request.Opening;
        cancellationToken.ThrowIfCancellationRequested();
        return request.Fails is { } failure ? throw failure : request;
    }

    /// <summary>Starts a run and waits until it reaches <paramref name="state"/>.</summary>
    private async Task<PairingStatus> PairAsync(string url, string fingerprint, string state)
    {
        var done = WaitForAsync(s => s.State == state);
        await _client.Begin(url, fingerprint, null);
        return await done;
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

    /// <summary>A pairing request on a fake server: it opens when <see cref="Opening"/> completes, and approves with <see cref="Approval"/>, or waits.</summary>
    private sealed class FakeRequest : IPairingRequest
    {
        public Task Opening { get; init; } = Task.CompletedTask;

        public Exception? Fails { get; init; }

        public Task<SkakelCredentials> Approval { get; init; } = new TaskCompletionSource<SkakelCredentials>().Task;

        /// <summary>The answer comes even after the run stopped waiting, as one already on its way does.</summary>
        public bool IgnoresCancel { get; init; }

        public bool Disposed { get; private set; }

        public string Code => "482913";

        public DateTimeOffset ExpiresAt => DateTimeOffset.UtcNow.AddMinutes(5);

        public static FakeRequest Approving(string clientId, string secret) => new() { Approval = Task.FromResult(new SkakelCredentials(clientId, secret)) };

        public Task<SkakelCredentials> WaitAsync(CancellationToken cancellationToken) =>
            IgnoresCancel ? Approval : Approval.WaitAsync(cancellationToken);

        public ValueTask DisposeAsync()
        {
            Disposed = true;
            return ValueTask.CompletedTask;
        }
    }
}
