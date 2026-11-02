using System.Text.Json;
using Microsoft.Extensions.Logging.Abstractions;

namespace Koppel.Core.Configuration.Tests;

public sealed class JsonConfigFileTests : IDisposable
{
    private readonly string _directory = Path.Combine(Path.GetTempPath(), "koppel-core-tests", Guid.NewGuid().ToString("N"));

    private string FilePath => Path.Combine(_directory, "sample.json");

    private static CancellationToken Ct => TestContext.Current.CancellationToken;

    public void Dispose()
    {
        if (Directory.Exists(_directory))
        {
            Directory.Delete(_directory, recursive: true);
        }
    }

    [Fact]
    public async Task Start_seeds_the_defaults_when_the_file_is_missing()
    {
        using var file = Create();

        await file.StartAsync(Ct);

        File.Exists(FilePath).Should().BeTrue();
        Read().Should().Be(new Sample("default", 1));
    }

    [Fact]
    public async Task Start_refuses_a_file_that_is_not_valid_JSON_and_leaves_it_alone()
    {
        Directory.CreateDirectory(_directory);
        await File.WriteAllTextAsync(FilePath, "{ not json", Ct);
        var before = await File.ReadAllBytesAsync(FilePath, Ct);
        using var file = Create();

        var start = () => file.StartAsync(Ct);

        (await start.Should().ThrowAsync<ConfigFileException>()).Which.Message.Should().Contain(FilePath);
        (await File.ReadAllBytesAsync(FilePath, Ct)).Should().Equal(before);
        Directory.GetFiles(_directory).Should().ContainSingle();
    }

    [Fact]
    public async Task Start_refuses_a_file_it_cannot_decrypt()
    {
        var path = FilePath;
        using (var box = new Security.SecretBox(Path.Combine(_directory, "keys-a"), "test"))
        using (var file = new JsonConfigFile<Sample>(path, new Sample("default", 1), JsonSerializerOptions.Web, NullLogger.Instance, box.AsFileCodec()))
        {
            await file.StartAsync(Ct);
            await file.UpdateAsync(_ => new Sample("secret", 2), Ct);
        }

        var before = await File.ReadAllBytesAsync(path, Ct);
        using var other = new Security.SecretBox(Path.Combine(_directory, "keys-b"), "test");
        using var again = new JsonConfigFile<Sample>(path, new Sample("default", 1), JsonSerializerOptions.Web, NullLogger.Instance, other.AsFileCodec());

        var start = () => again.StartAsync(Ct);

        (await start.Should().ThrowAsync<ConfigFileException>()).Which.Reason.Should().Contain("decrypted");
        (await File.ReadAllBytesAsync(path, Ct)).Should().Equal(before);
    }

    [Fact]
    public async Task Start_refuses_a_file_that_fails_validation()
    {
        Directory.CreateDirectory(_directory);
        await File.WriteAllTextAsync(FilePath, """{ "name": "x", "number": -5 }""", Ct);
        using var file = Create(validate: Positive);

        var start = () => file.StartAsync(Ct);

        (await start.Should().ThrowAsync<ConfigFileException>()).Which.Reason.Should().Contain("number must be positive");
        Read().Should().Be(new Sample("x", -5));
    }

    [Fact]
    public async Task Reload_of_an_invalid_edit_keeps_the_last_good_value_and_reports_it()
    {
        using var file = Create(validate: Positive);
        await file.StartAsync(Ct);
        var raised = 0;
        file.Changed += _ =>
        {
            Interlocked.Increment(ref raised);
            return Task.CompletedTask;
        };

        await File.WriteAllTextAsync(FilePath, """{ "name": "x", "number": -5 }""", Ct);
        await WaitForAsync(() => file.Problem is not null);

        file.Problem.Should().Contain("number must be positive");
        file.Current.Should().Be(new Sample("default", 1));
        raised.Should().Be(0);

        await File.WriteAllTextAsync(FilePath, """{ "name": "fixed", "number": 7 }""", Ct);
        await WaitForAsync(() => file.Problem is null && file.Current.Number == 7);
        raised.Should().Be(1);
    }

    [Fact]
    public async Task Concurrent_updates_reach_subscribers_in_write_order()
    {
        using var file = Create();
        await file.StartAsync(Ct);
        var seen = new List<int>();
        file.Changed += async value =>
        {
            // A slow handler must not let a later change overtake an earlier one.
            if (value.Number % 3 == 0)
            {
                await Task.Delay(20);
            }

            lock (seen)
            {
                seen.Add(value.Number);
            }
        };

        var updates = Enumerable.Range(1, 30)
            .Select(_ => Task.Run(() => file.UpdateAsync(current => current with { Number = current.Number + 1 }, Ct), Ct))
            .ToArray();
        await Task.WhenAll(updates);

        seen.Should().HaveCount(30).And.BeInAscendingOrder();
        file.Current.Number.Should().Be(31);
    }

    [Fact]
    public async Task A_handler_that_updates_another_file_does_not_deadlock()
    {
        using var first = Create();
        using var second = new JsonConfigFile<Sample>(Path.Combine(_directory, "second.json"), new Sample("default", 1), JsonSerializerOptions.Web, NullLogger.Instance);
        await first.StartAsync(Ct);
        await second.StartAsync(Ct);
        first.Changed += value => second.UpdateAsync(current => current with { Number = value.Number }, Ct);

        await first.UpdateAsync(current => current with { Number = 9 }, Ct).WaitAsync(TimeSpan.FromSeconds(5), Ct);

        second.Current.Number.Should().Be(9);
    }

    [Fact]
    public async Task A_handler_that_updates_the_same_file_does_not_deadlock()
    {
        using var file = Create();
        await file.StartAsync(Ct);
        file.Changed += value => value.Number == 2
            ? file.UpdateAsync(current => current with { Number = 3 }, Ct)
            : Task.CompletedTask;

        await file.UpdateAsync(current => current with { Number = 2 }, Ct).WaitAsync(TimeSpan.FromSeconds(5), Ct);

        file.Current.Number.Should().Be(3);
    }

    [Fact]
    public async Task A_nested_update_is_delivered_after_the_running_delivery_in_write_order_for_every_subscriber()
    {
        using var file = Create();
        await file.StartAsync(Ct);
        var first = new List<int>();
        var second = new List<int>();
        var enteredTwo = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var releaseTwo = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        file.Changed += async value =>
        {
            lock (first)
            {
                first.Add(value.Number);
            }

            if (value.Number == 2)
            {
                enteredTwo.SetResult();
                await releaseTwo.Task;

                // Written while 2 is still being delivered, after the 10 that is already waiting.
                await file.UpdateAsync(current => current with { Number = 3 }, Ct);
            }
        };
        file.Changed += value =>
        {
            lock (second)
            {
                second.Add(value.Number);
            }

            return Task.CompletedTask;
        };

        var two = file.UpdateAsync(current => current with { Number = 2 }, Ct);
        await enteredTwo.Task.WaitAsync(TimeSpan.FromSeconds(5), Ct);
        var ten = file.UpdateAsync(current => current with { Number = 10 }, Ct);
        await WaitForAsync(() => file.Current.Number == 10);
        releaseTwo.SetResult();
        await Task.WhenAll(two, ten).WaitAsync(TimeSpan.FromSeconds(5), Ct);
        await WaitForAsync(() => file.Current.Number == 3 && second.Count == 3);

        first.Should().Equal(2, 10, 3);
        second.Should().Equal(2, 10, 3);
    }

    [Fact]
    public async Task A_file_opened_to_start_with_defaults_survives_an_unusable_file_and_leaves_it_alone()
    {
        Directory.CreateDirectory(_directory);
        await File.WriteAllTextAsync(FilePath, "{ not json", Ct);
        var before = await File.ReadAllBytesAsync(FilePath, Ct);
        using var file = Tolerant();

        await file.StartAsync(Ct);

        file.Current.Should().Be(new Sample("default", 1));
        file.Problem.Should().Contain("not valid JSON");
        (await File.ReadAllBytesAsync(FilePath, Ct)).Should().Equal(before);
        Directory.GetFiles(_directory).Should().ContainSingle();
    }

    [Fact]
    public async Task A_change_over_an_unusable_file_moves_it_aside_and_writes_a_fresh_one()
    {
        Directory.CreateDirectory(_directory);
        await File.WriteAllTextAsync(FilePath, "{ not json", Ct);
        using var file = Tolerant();
        await file.StartAsync(Ct);

        await file.UpdateAsync(current => current with { Number = 5 }, Ct);

        Read().Should().Be(new Sample("default", 5));
        file.Problem.Should().BeNull();
        var aside = Directory.GetFiles(_directory, "sample.json.unreadable-*").Should().ContainSingle().Subject;
        (await File.ReadAllTextAsync(aside, Ct)).Should().Be("{ not json");
    }

    [Fact]
    public async Task A_change_that_leaves_the_defaults_as_they_are_still_replaces_an_unusable_file()
    {
        Directory.CreateDirectory(_directory);
        await File.WriteAllTextAsync(FilePath, "{ not json", Ct);
        using var file = Tolerant();
        await file.StartAsync(Ct);

        await file.UpdateAsync(current => current, Ct);

        Read().Should().Be(new Sample("default", 1));
        Directory.GetFiles(_directory, "sample.json.unreadable-*").Should().ContainSingle();
    }

    [Fact]
    public async Task Fixing_an_unusable_file_by_hand_clears_the_problem_and_is_applied()
    {
        Directory.CreateDirectory(_directory);
        await File.WriteAllTextAsync(FilePath, "{ not json", Ct);
        using var file = Tolerant();
        await file.StartAsync(Ct);

        await File.WriteAllTextAsync(FilePath, """{ "name": "fixed", "number": 7 }""", Ct);
        await WaitForAsync(() => file.Problem is null && file.Current.Number == 7);

        Directory.GetFiles(_directory, "sample.json.unreadable-*").Should().BeEmpty();
    }

    [Fact]
    public async Task An_update_that_changes_nothing_raises_once_and_does_not_write()
    {
        using var file = Create();
        await file.StartAsync(Ct);
        var raised = 0;
        file.Changed += _ =>
        {
            Interlocked.Increment(ref raised);
            return Task.CompletedTask;
        };

        // A write would move the time forward from this fixed point, so no waiting is needed to tell.
        var longAgo = new DateTime(2020, 1, 1, 0, 0, 0, DateTimeKind.Utc);
        File.SetLastWriteTimeUtc(FilePath, longAgo);

        await file.UpdateAsync(current => current, Ct);

        raised.Should().Be(1);
        File.GetLastWriteTimeUtc(FilePath).Should().Be(longAgo);
    }

    [Fact]
    public async Task A_hand_edit_is_not_lost_to_a_following_update()
    {
        using var file = Create();
        await file.StartAsync(Ct);

        // Written before the watcher reloads it: the update must still see it.
        await File.WriteAllTextAsync(FilePath, """{ "name": "by hand", "number": 5 }""", Ct);
        var result = await file.UpdateAsync(current => current with { Number = current.Number + 1 }, Ct);

        result.Should().Be(new Sample("by hand", 6));
        Read().Should().Be(new Sample("by hand", 6));
    }

    [Fact]
    public async Task A_throwing_handler_does_not_stop_the_others()
    {
        using var file = Create();
        await file.StartAsync(Ct);
        var reached = false;
        file.Changed += _ => throw new InvalidOperationException("boom");
        file.Changed += _ =>
        {
            reached = true;
            return Task.CompletedTask;
        };

        await file.UpdateAsync(current => current with { Number = 2 }, Ct);

        reached.Should().BeTrue();
        file.Current.Number.Should().Be(2);
    }

    [Fact]
    public async Task Temp_files_are_not_left_behind_and_stale_ones_are_removed_at_start()
    {
        Directory.CreateDirectory(_directory);
        var stale = FilePath + ".0123456789abcdef.tmp";
        await File.WriteAllTextAsync(stale, "left by a crash", Ct);
        var unrelated = Path.Combine(_directory, "other.json.tmp");
        await File.WriteAllTextAsync(unrelated, "not ours", Ct);
        using var file = Create();

        await file.StartAsync(Ct);
        await file.UpdateAsync(current => current with { Number = 2 }, Ct);

        File.Exists(stale).Should().BeFalse();
        File.Exists(unrelated).Should().BeTrue();
        Directory.GetFiles(_directory, "sample.json.*.tmp").Should().BeEmpty();
    }

    [Fact]
    public async Task A_failed_write_deletes_its_temp_file()
    {
        using var file = Create();
        await file.StartAsync(Ct);
        file.Stop();

        // A directory where the file belongs: the move over it fails every time.
        File.Delete(FilePath);
        Directory.CreateDirectory(FilePath);
        var update = () => file.UpdateAsync(current => current with { Number = 2 }, Ct);

        await update.Should().ThrowAsync<Exception>();
        Directory.GetFiles(_directory, "sample.json.*.tmp").Should().BeEmpty();
    }

    private static string? Positive(Sample sample) => sample.Number > 0 ? null : "number must be positive";

    private static async Task WaitForAsync(Func<bool> condition)
    {
        var deadline = DateTime.UtcNow.AddSeconds(10);
        while (!condition())
        {
            if (DateTime.UtcNow > deadline)
            {
                throw new TimeoutException("The condition did not become true.");
            }

            await Task.Delay(25, Ct);
        }
    }

    private JsonConfigFile<Sample> Create(Func<Sample, string?>? validate = null) =>
        new(FilePath, new Sample("default", 1), JsonSerializerOptions.Web, NullLogger.Instance, validate: validate);

    private JsonConfigFile<Sample> Tolerant() =>
        new(FilePath, new Sample("default", 1), JsonSerializerOptions.Web, NullLogger.Instance, startWithDefaultsWhenUnusable: true);

    private Sample Read() => JsonSerializer.Deserialize<Sample>(File.ReadAllText(FilePath), JsonSerializerOptions.Web)!;

    private sealed record Sample(string Name, int Number);
}
