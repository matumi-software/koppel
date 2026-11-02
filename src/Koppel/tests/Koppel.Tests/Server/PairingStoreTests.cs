using System.Net;
using System.Text.Json;
using Koppel.Core.Activity;
using Koppel.Core.Security;
using Koppel.Server;
using Microsoft.Extensions.Logging.Abstractions;

namespace Koppel.Tests.Server;

/// <summary>
/// The server's pairing requests on their own, against a manual clock: who may poll and withdraw,
/// how many requests a server takes, and what concurrent approvals and wrong codes do.
/// </summary>
public sealed class PairingStoreTests : IAsyncLifetime
{
    private readonly string _directory = Path.Combine(Path.GetTempPath(), "koppel-tests", Guid.NewGuid().ToString("N"));
    private readonly ManualTimeProvider _time = new();
    private SecretBox _box = null!;
    private ServerSecretsStore _secrets = null!;
    private PairingStore _store = null!;

    public async ValueTask InitializeAsync()
    {
        _box = new SecretBox(Path.Combine(_directory, "keys"), "test");
        _secrets = new ServerSecretsStore(new KoppelPaths(Path.Combine(_directory, "config"), Path.Combine(_directory, "keys")), _box, _time, NullLogger<ServerSecretsStore>.Instance);
        await _secrets.StartAsync(TestContext.Current.CancellationToken);
        _store = new PairingStore(_secrets, new InMemoryActivityStore(_time), _time, NullLogger<PairingStore>.Instance);
    }

    public ValueTask DisposeAsync()
    {
        _store.Dispose();
        _secrets.Dispose();
        _box.Dispose();
        if (Directory.Exists(_directory))
        {
            Directory.Delete(_directory, recursive: true);
        }

        return ValueTask.CompletedTask;
    }

    [Fact]
    public async Task Poll_and_withdraw_need_the_token_the_open_reply_carried()
    {
        var (opened, _) = await OpenAndRevealAsync();

        (await _store.PollAsync(opened.Id, null)).Should().BeNull();
        (await _store.PollAsync(opened.Id, "")).Should().BeNull();
        (await _store.PollAsync(opened.Id, new string('0', opened.PollToken.Length))).Should().BeNull();
        (await _store.WithdrawAsync(opened.Id, null)).Should().BeFalse();
        (await _store.WithdrawAsync(opened.Id, "wrong")).Should().BeFalse();
        _store.Pending.Should().HaveCount(1, "a refused withdraw leaves the request where it was");

        (await _store.PollAsync(opened.Id, opened.PollToken))!.State.Should().Be("waiting");
        (await _store.WithdrawAsync(opened.Id, opened.PollToken)).Should().BeTrue();
        _store.Pending.Should().BeEmpty();
    }

    [Fact]
    public async Task The_poll_token_is_random_per_request_and_never_in_what_the_ui_sees()
    {
        var (first, _) = await OpenAndRevealAsync(Address(1000));
        var (second, _) = await OpenAndRevealAsync(Address(1001));

        first.PollToken.Should().NotBe(second.PollToken);
        first.PollToken.Should().HaveLength(64, "32 random bytes as hex");
        var pushed = JsonSerializer.Serialize(_store.Pending);
        pushed.Should().NotContain(first.PollToken).And.NotContain(second.PollToken);
        typeof(PairingRequestView).GetProperties().Select(p => p.Name).Should().NotContain(n => n.Contains("Token", StringComparison.Ordinal));
    }

    [Fact]
    public async Task The_server_takes_a_fixed_number_of_opens_per_window_and_withdrawing_does_not_give_them_back()
    {
        for (var i = 0; i < PairingStore.OpensPerWindow; i++)
        {
            var opened = _store.Open("bench", Commitment(), Address(i));
            opened.Should().NotBeNull();
            (await _store.WithdrawAsync(opened!.Id, opened.PollToken)).Should().BeTrue();
        }

        _store.Open("bench", Commitment(), Address(99)).Should().BeNull("nothing is open, yet the budget is spent");

        _time.Advance(PairingStore.OpenWindow);

        _store.Open("bench", Commitment(), Address(99)).Should().NotBeNull("the window moved on");
    }

    [Fact]
    public void Expiry_does_not_give_opens_back_either()
    {
        for (var i = 0; i < PairingStore.OpensPerWindow; i++)
        {
            _store.Open("bench", Commitment(), Address(i)).Should().NotBeNull();
        }

        _time.Advance(PairingStore.UnrevealedLifetime + TimeSpan.FromSeconds(10));

        _store.Open("bench", Commitment(), Address(99)).Should().BeNull("those requests are gone, but they were spent");
    }

    [Fact]
    public void Requests_nobody_can_see_yet_do_not_count_against_the_cap()
    {
        for (var i = 0; i < PairingStore.MaxOpen; i++)
        {
            _store.Open("hidden", Commitment(), Address(i)).Should().NotBeNull();
        }

        _store.Open("eleventh", Commitment(), Address(50)).Should().NotBeNull("unrevealed requests are not waiting for anyone");
    }

    [Fact]
    public async Task Requests_a_person_can_see_count_against_the_cap()
    {
        for (var i = 0; i < PairingStore.MaxOpen; i++)
        {
            await OpenAndRevealAsync(Address(100 + i));
        }

        _store.Open("one too many", Commitment(), Address(200)).Should().BeNull();
    }

    [Fact]
    public async Task A_request_that_never_reveals_is_dropped_after_a_short_while()
    {
        var opened = _store.Open("slow", PairingCode.Commit("n"), IPAddress.Parse("10.0.0.1"))!;

        _time.Advance(PairingStore.UnrevealedLifetime + TimeSpan.FromSeconds(1));

        (await _store.RevealAsync(opened.Id, "n")).Should().BeFalse();
        (await _store.PollAsync(opened.Id, opened.PollToken)).Should().BeNull("the sweep removed it");
    }

    [Fact]
    public void Addresses_count_per_ipv4_address_and_per_ipv6_slash_64()
    {
        var one = IPAddress.Parse("2001:db8:1:1::1");
        for (var i = 0; i < PairingStore.MaxOpenPerAddress; i++)
        {
            _store.Open("v6", Commitment(), IPAddress.Parse($"2001:db8:1:1::{i + 1:x}")).Should().NotBeNull();
        }

        _store.Open("v6", Commitment(), IPAddress.Parse("2001:db8:1:1:ffff::9")).Should().BeNull("same /64, different address");
        _store.Open("v6", Commitment(), IPAddress.Parse("2001:db8:1:2::1")).Should().NotBeNull("another /64");
        _store.Open("v4", Commitment(), IPAddress.Parse("10.0.0.1")).Should().NotBeNull();
        PairingStore.AddressKey(one).Should().Be(PairingStore.AddressKey(IPAddress.Parse("2001:db8:1:1:aaaa:bbbb:cccc:dddd")));
        PairingStore.AddressKey(IPAddress.Parse("::ffff:10.0.0.1")).Should().Be(PairingStore.AddressKey(IPAddress.Parse("10.0.0.1")));
    }

    [Fact]
    public async Task Two_people_approving_at_once_make_one_client()
    {
        var (opened, code) = await OpenAndRevealAsync();

        var results = await Task.WhenAll(Enumerable.Range(0, 8).Select(_ => Task.Run(() => _store.ApproveAsync(opened.Id, code, CancellationToken.None))));

        results.Count(r => r == true).Should().Be(1);
        results.Count(r => r is null).Should().Be(7, "the request is no longer waiting");
        _secrets.Clients.Should().HaveCount(1);
        var poll = (await _store.PollAsync(opened.Id, opened.PollToken))!;
        poll.State.Should().Be("approved");
        poll.ClientId.Should().Be(_secrets.Clients[0].Id);
        poll.ClientSecret.Should().NotBeNullOrEmpty();
        (await _store.PollAsync(opened.Id, opened.PollToken)).Should().BeNull("credentials are handed over once");
    }

    [Fact]
    public async Task Wrong_codes_typed_at_once_are_all_counted()
    {
        var (opened, code) = await OpenAndRevealAsync();
        var wrong = code == "000000" ? "111111" : "000000";

        var results = await Task.WhenAll(Enumerable.Range(0, 8).Select(_ => Task.Run(() => _store.ApproveAsync(opened.Id, wrong, CancellationToken.None))));

        results.Count(r => r == false).Should().Be(PairingStore.MaxWrongCodes, "no count lost and none past the limit");
        results.Count(r => r is null).Should().Be(8 - PairingStore.MaxWrongCodes);
        (await _store.ApproveAsync(opened.Id, code, CancellationToken.None)).Should().BeNull("the right code is too late");
        _secrets.Clients.Should().BeEmpty();
    }

    [Fact]
    public async Task A_request_that_runs_out_of_time_leaves_the_list_and_the_ui_is_told_without_anyone_calling()
    {
        await OpenAndRevealAsync();
        var pushes = new List<int>();
        _store.Changed += list =>
        {
            pushes.Add(list.Count);
            return Task.CompletedTask;
        };

        _time.Advance(PairingStore.Lifetime + PairingStore.SweepInterval);

        pushes.Should().Contain(0);
        _store.Pending.Should().BeEmpty();
    }

    [Fact]
    public async Task Three_wrong_codes_keep_the_request_listed_as_denied_for_a_moment()
    {
        var (opened, code) = await OpenAndRevealAsync();
        var wrong = code == "000000" ? "111111" : "000000";
        var pushed = new List<IReadOnlyList<PairingRequestView>>();
        _store.Changed += list =>
        {
            pushed.Add(list);
            return Task.CompletedTask;
        };

        for (var i = 0; i < PairingStore.MaxWrongCodes; i++)
        {
            await _store.ApproveAsync(opened.Id, wrong, CancellationToken.None);
        }

        var view = _store.Pending.Should().ContainSingle().Subject;
        view.State.Should().Be("denied");
        view.Message.Should().Contain("wrong three times");
        pushed.Last().Should().ContainSingle(v => v.State == "denied");
        (await _store.PollAsync(opened.Id, opened.PollToken))!.State.Should().Be("denied", "the client hears it too");

        _time.Advance(PairingStore.DeniedVisibleFor + PairingStore.SweepInterval);

        _store.Pending.Should().BeEmpty();
        pushed.Last().Should().BeEmpty("the list is pushed again once the denied entry goes");
    }

    [Fact]
    public async Task Refusing_a_request_takes_it_off_the_list_at_once()
    {
        var (opened, _) = await OpenAndRevealAsync();

        (await _store.DenyAsync(opened.Id)).Should().BeTrue();

        _store.Pending.Should().BeEmpty();
        (await _store.PollAsync(opened.Id, opened.PollToken))!.State.Should().Be("denied");
    }

    private async Task<(PairingOpened Opened, string Code)> OpenAndRevealAsync(IPAddress? address = null)
    {
        var nonce = PairingCode.NewNonce();
        var opened = _store.Open("bench", PairingCode.Commit(nonce), address ?? Address(0))!;
        (await _store.RevealAsync(opened.Id, nonce)).Should().BeTrue();
        return (opened, PairingCode.Compute(_secrets.Fingerprint, nonce, opened.ServerNonce));
    }

    private static string Commitment() => PairingCode.Commit(PairingCode.NewNonce());

    private static IPAddress Address(int i) => IPAddress.Parse($"10.1.{i / 250}.{i % 250 + 1}");
}
