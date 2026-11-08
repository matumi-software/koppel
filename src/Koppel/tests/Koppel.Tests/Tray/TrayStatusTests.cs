using Koppel.Settings;
using Koppel.Tray;
using Koppel.Usb;

namespace Koppel.Tests.Tray;

public sealed class TrayStatusTests
{
    private static readonly ServerDevices NoServerDevices = new([], null);
    private static readonly ClientDevices Unconfigured = new("unconfigured", null, [], null);

    private static DeviceView Device(string busId, string state) =>
        new(busId, "046d:c52b", "USB Receiver", "Logitech", "full", state, null, null, null);

    private static ClientDevices Linked(string link, params bool[] attached) =>
        new(link, null, [.. attached.Select((here, i) => new ClientDeviceView(Device($"1-{i + 1}", "shared"), here, null, null, null))], null);

    private static TrayFacts Facts(KoppelRole role, ClientDevices? client = null, ServerDevices? server = null, string? host = "t520-client",
        bool passwordSet = true, bool setupOpen = false) =>
        new(role, passwordSet, setupOpen, client ?? Unconfigured, host, server ?? NoServerDevices);

    [Fact]
    public void A_linked_client_names_its_server_and_counts_what_it_attached() =>
        TrayStatus.Describe(Facts(KoppelRole.Client, Linked("connected", true, false, true)))
            .Should().Be(new TrayState("Koppel: linked to t520-client, 2 devices attached", false));

    [Fact]
    public void One_device_is_singular() =>
        TrayStatus.Describe(Facts(KoppelRole.Client, Linked("connected", true))).Tip
            .Should().Be("Koppel: linked to t520-client, 1 device attached");

    [Fact]
    public void A_client_that_cannot_reach_its_server_needs_attention() =>
        TrayStatus.Describe(Facts(KoppelRole.Client, Linked("error")))
            .Should().Be(new TrayState("Koppel: cannot reach t520-client", true));

    [Fact]
    public void Connecting_and_no_server_yet_need_no_attention()
    {
        TrayStatus.Describe(Facts(KoppelRole.Client, Linked("connecting"))).Should().Be(new TrayState("Koppel: connecting to t520-client", false));
        TrayStatus.Describe(Facts(KoppelRole.Client, host: null)).Should().Be(new TrayState("Koppel: no server yet", false));
    }

    [Fact]
    public void A_server_counts_what_it_shares_and_what_is_in_use()
    {
        var server = new ServerDevices([Device("1-1", "shared"), Device("1-2", "attached"), Device("1-3", "offline"), Device("1-4", "local")], null);

        TrayStatus.Describe(Facts(KoppelRole.Server, server: server))
            .Should().Be(new TrayState("Koppel: server, 3 devices shared, 1 in use", false));
    }

    [Fact]
    public void A_server_that_cannot_share_needs_attention() =>
        TrayStatus.Describe(Facts(KoppelRole.Server, server: new ServerDevices([], "usbip-host is not loaded")))
            .Should().Be(new TrayState("Koppel: server, sharing is not available", true));

    [Fact]
    public void Both_roles_get_a_line_each()
    {
        var state = TrayStatus.Describe(Facts(KoppelRole.Both, Linked("error"), new ServerDevices([Device("1-1", "shared")], null)));

        state.Should().Be(new TrayState("Koppel\nServer, 1 device shared, 0 in use\nCannot reach t520-client", true));
    }

    [Fact]
    public void Before_the_password_is_set_it_says_setup_is_waiting_or_has_timed_out()
    {
        TrayStatus.Describe(Facts(KoppelRole.Server, passwordSet: false, setupOpen: true))
            .Should().Be(new TrayState("Koppel: setting up. Open Koppel to set the password.", true));
        TrayStatus.Describe(Facts(KoppelRole.Server, passwordSet: false, setupOpen: false))
            .Should().Be(new TrayState("Koppel: setup has timed out. Restart Koppel to set it up.", true));
    }

    [Fact]
    public void A_long_server_name_is_cut_to_fit_the_tooltip()
    {
        var tip = TrayStatus.Describe(Facts(KoppelRole.Client, Linked("connected", true), host: new string('a', 200))).Tip;

        tip.Length.Should().Be(TrayStatus.MaxTip);
        tip.Should().EndWith("...");
    }

    [Fact]
    public void The_wire_carries_a_state_both_ways()
    {
        var state = new TrayState("Koppel: cannot reach \"lab\"\nsecond line", true);

        TrayWire.ReadState(TrayWire.State(state)).Should().Be(state);
    }

    [Theory]
    [InlineData("")]
    [InlineData("not json")]
    [InlineData("[1,2]")]
    [InlineData("{\"type\":\"notice\",\"title\":\"x\"}")]
    [InlineData("{\"type\":\"state\"}")]
    [InlineData("{\"type\":\"state\",\"tip\":5}")]
    public void A_tray_skips_a_line_it_cannot_read(string line) =>
        TrayWire.ReadState(line).Should().BeNull();

    [Fact]
    public void A_tray_reads_a_state_with_fields_it_does_not_know() =>
        TrayWire.ReadState("{\"type\":\"state\",\"tip\":\"Koppel\",\"attention\":false,\"badge\":3}")
            .Should().Be(new TrayState("Koppel", false));

    [Fact]
    public void The_dot_sits_in_the_lower_right_and_leaves_the_rest_alone()
    {
        const int size = 32;
        var pixels = Enumerable.Repeat(0xFF00AA00u, size * size).ToArray();

        TrayAttentionDot.Draw(pixels, size);

        pixels[0].Should().Be(0xFF00AA00u);
        pixels[(size * size) - 1 - (size * 4) - 4].Should().Be(0xFFF59E0Bu); // well inside the dot
    }
}
