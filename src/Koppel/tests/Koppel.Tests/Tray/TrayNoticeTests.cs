using Koppel.Core.Activity;
using Koppel.Settings;
using Koppel.Tray;

namespace Koppel.Tests.Tray;

public sealed class TrayNoticeTests
{
    private static readonly KoppelSettings AllOn = KoppelSettings.Default;

    private static ActivityEvent Activity(ActivityReason reason, ActivityOutcome outcome, string summary, string? detail = null) =>
        new("id", DateTimeOffset.UnixEpoch, new EntityRef("x", "x", "link"), reason, outcome, summary, detail);

    private static ActivityEvent LinkDown => Activity(ActivityReason.LinkDown, ActivityOutcome.Failed, "Lost the link to lab", "Connection refused");

    private static ActivityEvent LinkUp => Activity(ActivityReason.LinkUp, ActivityOutcome.Succeeded, "Linked to lab");

    [Fact]
    public void A_lost_link_is_told_once_and_its_return_only_after_that()
    {
        var rules = new TrayNoticeRules();

        rules.FromActivity(LinkUp, AllOn).Should().BeNull("the first link after a start is not news");
        rules.FromActivity(LinkDown, AllOn).Should().Be(new TrayNotice(
            "Koppel lost the link to the server",
            "Lost the link to lab. Koppel keeps trying, and attaches your devices again when it is back.",
            "#attach"));
        rules.FromActivity(LinkUp, AllOn).Should().Be(new TrayNotice("Koppel is linked again", "Linked to lab again.", "#attach"));
        rules.FromActivity(LinkUp, AllOn).Should().BeNull();
    }

    [Fact]
    public void A_failed_attach_names_the_device_and_why()
    {
        var notice = new TrayNoticeRules().FromActivity(
            Activity(ActivityReason.Attached, ActivityOutcome.Failed, "USB Receiver was not attached", "The server is not sharing it"), AllOn);

        notice.Should().Be(new TrayNotice("A device was not attached", "USB Receiver was not attached: The server is not sharing it", "#attach"));
    }

    [Fact]
    public void Other_activity_is_not_a_notice()
    {
        var rules = new TrayNoticeRules();

        rules.FromActivity(Activity(ActivityReason.Attached, ActivityOutcome.Succeeded, "USB Receiver is attached here"), AllOn).Should().BeNull();
        rules.FromActivity(Activity(ActivityReason.SignInFailed, ActivityOutcome.Failed, "Wrong password"), AllOn).Should().BeNull();
    }

    [Fact]
    public void Each_kind_can_be_turned_off()
    {
        var rules = new TrayNoticeRules();
        var off = AllOn with { NotifyLink = false, NotifyAttachFailures = false, NotifyPairing = false };

        rules.FromActivity(LinkDown, off).Should().BeNull();
        rules.FromActivity(LinkUp, off).Should().BeNull();
        rules.FromActivity(Activity(ActivityReason.Attached, ActivityOutcome.Failed, "USB Receiver was not attached", "x"), off).Should().BeNull();
        rules.FromPairing([new("r1", "bench-pc", "waiting")], off).Should().BeEmpty();
    }

    [Fact]
    public void A_pairing_request_is_told_once_when_it_arrives()
    {
        var rules = new TrayNoticeRules();

        rules.FromPairing([new("r1", "bench-pc", "waiting")], AllOn).Should().Equal(
            new TrayNotice("A client wants to pair", "bench-pc is waiting. Open Clients and type the code it shows.", "#clients"));
        rules.FromPairing([new("r2", "laptop", "waiting"), new("r1", "bench-pc", "waiting")], AllOn)
            .Should().ContainSingle().Which.Text.Should().StartWith("laptop");
        rules.FromPairing([new("r2", "laptop", "denied")], AllOn).Should().BeEmpty();
    }

    [Fact]
    public void A_denied_request_is_not_a_notice() =>
        new TrayNoticeRules().FromPairing([new("r1", "bench-pc", "denied")], AllOn).Should().BeEmpty();

    [Fact]
    public void A_notice_crosses_the_wire_with_its_page()
    {
        var notice = new TrayNotice("A client wants to pair", "bench-pc is waiting.", "#clients");

        TrayWire.ReadNotice(TrayWire.Notice(notice)).Should().Be(notice);
        TrayWire.ReadState(TrayWire.Notice(notice)).Should().BeNull();
        TrayWire.ReadNotice(TrayWire.State(TrayState.Plain)).Should().BeNull();
    }

    [Theory]
    [InlineData("https://evil.example/")]
    [InlineData("#clients/../x")]
    [InlineData("#Clients")]
    [InlineData("#")]
    [InlineData("javascript:alert(1)")]
    public void A_page_that_is_not_one_of_the_UIs_opens_the_UI_itself(string page) =>
        TrayWire.ReadNotice($$"""{"type":"notice","title":"t","text":"x","page":"{{page}}"}""")!.Page.Should().BeEmpty();

    [Fact]
    public void A_long_notice_is_cut_to_what_Windows_shows()
    {
        var notice = TrayWire.ReadNotice(TrayWire.Notice(new TrayNotice(new string('t', 100), new string('x', 400), "#attach")))!;

        notice.Title.Length.Should().Be(TrayWire.MaxTitle);
        notice.Text.Length.Should().Be(TrayWire.MaxText);
    }
}
