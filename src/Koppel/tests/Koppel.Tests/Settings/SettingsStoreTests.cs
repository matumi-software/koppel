using System.Text.Json;
using Koppel.Settings;
using Microsoft.Extensions.Logging.Abstractions;

namespace Koppel.Tests.Settings;

public sealed class SettingsStoreTests : IDisposable
{
    private readonly string _directory = Path.Combine(Path.GetTempPath(), "koppel-tests", Guid.NewGuid().ToString("N"));

    public void Dispose()
    {
        if (Directory.Exists(_directory))
        {
            Directory.Delete(_directory, recursive: true);
        }
    }

    [Fact]
    public async Task Start_writes_defaults_when_no_file_exists()
    {
        using var store = CreateStore();

        await store.StartAsync(TestContext.Current.CancellationToken);

        ReadFile().Should().Be(KoppelSettings.Default);
    }

    [Fact]
    public async Task Apply_writes_the_file_before_raising_Changed()
    {
        using var store = CreateStore();
        await store.StartAsync(TestContext.Current.CancellationToken);
        KoppelSettings? onDiskWhenRaised = null;
        store.Changed += _ =>
        {
            onDiskWhenRaised = ReadFile();
            return Task.CompletedTask;
        };

        await store.ApplyAsync(new SettingsPatch(null, ShareOnStartup: false, null), TestContext.Current.CancellationToken);

        onDiskWhenRaised.Should().NotBeNull();
        onDiskWhenRaised!.ShareOnStartup.Should().BeFalse();
        store.Current.ShareOnStartup.Should().BeFalse();
        File.Exists(SettingsFile + ".tmp").Should().BeFalse();
    }

    [Fact]
    public async Task Hand_edit_is_picked_up_and_raises_Changed()
    {
        using var store = CreateStore();
        await store.StartAsync(TestContext.Current.CancellationToken);
        var raised = new TaskCompletionSource<KoppelSettings>(TaskCreationOptions.RunContinuationsAsynchronously);
        store.Changed += settings =>
        {
            raised.TrySetResult(settings);
            return Task.CompletedTask;
        };

        var edited = KoppelSettings.Default with { ListenPort = 4000 };
        await File.WriteAllTextAsync(SettingsFile, JsonSerializer.Serialize(edited, KoppelJson.Files), TestContext.Current.CancellationToken);

        var result = await raised.Task.WaitAsync(TimeSpan.FromSeconds(5), TestContext.Current.CancellationToken);
        result.ListenPort.Should().Be(4000);
        store.Current.ListenPort.Should().Be(4000);
    }

    private string SettingsFile => Path.Combine(_directory, "settings.json");

    private SettingsStore CreateStore() => new(new KoppelPaths(_directory, Path.Combine(_directory, "keys")), NullLogger<SettingsStore>.Instance);

    private KoppelSettings ReadFile() =>
        JsonSerializer.Deserialize<KoppelSettings>(File.ReadAllText(SettingsFile), KoppelJson.Files)!;
}
