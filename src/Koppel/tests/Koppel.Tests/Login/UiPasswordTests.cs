using Koppel.Login;
using Koppel.Web;

namespace Koppel.Tests.Login;

public sealed class UiPasswordTests
{
    [Theory]
    [InlineData("", false)]
    [InlineData("short", false)]
    [InlineData("123456789", false)]
    [InlineData("KOPPEL", false)]
    [InlineData("pAsSwOrD", false)]
    [InlineData("1234567890", true)]
    [InlineData("correct horse battery", true)]
    public void Validate_applies_the_length_and_obvious_word_rules(string password, bool acceptable) =>
        (UiPassword.Validate(password) is null).Should().Be(acceptable);

    [Fact]
    public void Validate_refuses_a_very_long_password()
    {
        UiPassword.Validate(new string('a', UiPassword.MaxLength + 1)).Should().NotBeNull();
        UiPassword.Validate(new string('a', UiPassword.MaxLength)).Should().BeNull();
    }

    [Fact]
    public void Hash_keeps_its_parameters_and_verifies_only_the_password_it_was_made_from()
    {
        var hash = UiPassword.Hash("correct horse battery");

        var parts = hash.Split('$');
        parts[0].Should().Be("pbkdf2-sha256");
        int.Parse(parts[1], System.Globalization.CultureInfo.InvariantCulture).Should().BeGreaterThanOrEqualTo(600_000);
        Convert.FromBase64String(parts[2]).Should().HaveCount(16);
        UiPassword.Verify("correct horse battery", hash).Should().BeTrue();
        UiPassword.Verify("correct horse batterY", hash).Should().BeFalse();
        UiPassword.Hash("correct horse battery").Should().NotBe(hash, "each hash has its own salt");
    }

    [Theory]
    [InlineData("")]
    [InlineData("sha256$abc")]
    [InlineData("pbkdf2-sha256$1000$AAAAAAAAAAAAAAAAAAAAAA==$AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=")]
    [InlineData("pbkdf2-sha256$600000$not-base64$also-not")]
    public void A_stored_hash_in_any_other_shape_is_not_accepted(string stored) =>
        UiPassword.IsWellFormed(stored).Should().BeFalse();
}

public sealed class UiLoginThrottleTests
{
    [Fact]
    public void A_key_is_locked_after_five_failures_for_a_minute_and_then_for_twice_as_long()
    {
        var clock = new ManualTimeProvider();
        var throttle = new UiLoginThrottle(clock);

        for (var i = 0; i < UiLoginThrottle.FreeFailures - 1; i++)
        {
            throttle.Fail("ip");
            throttle.LockedFor("ip").Should().BeNull();
        }

        throttle.Fail("ip");
        throttle.LockedFor("ip").Should().Be(TimeSpan.FromMinutes(1));
        throttle.LockedFor("another").Should().BeNull();

        clock.Advance(TimeSpan.FromSeconds(61));
        throttle.LockedFor("ip").Should().BeNull();
        throttle.Fail("ip");
        throttle.LockedFor("ip").Should().Be(TimeSpan.FromMinutes(2));
    }

    [Fact]
    public void A_success_clears_the_failures()
    {
        var throttle = new UiLoginThrottle(new ManualTimeProvider());
        for (var i = 0; i < UiLoginThrottle.FreeFailures; i++)
        {
            throttle.Fail("ip");
        }

        throttle.Clear("ip");

        throttle.LockedFor("ip").Should().BeNull();
    }

    [Fact]
    public void The_lock_never_grows_past_an_hour()
    {
        var throttle = new UiLoginThrottle(new ManualTimeProvider());
        for (var i = 0; i < 60; i++)
        {
            throttle.Fail("ip");
        }

        throttle.LockedFor("ip").Should().Be(TimeSpan.FromHours(1));
    }
}

public sealed class SignInRuleTests
{
    [Theory]
    [InlineData("/api/settings", true)]
    [InlineData("/api/server/clients", true)]
    [InlineData("/api/client/devices", true)]
    [InlineData("/api/hubs/ui", true)]
    [InlineData("/api/a-route-added-tomorrow", true)]
    [InlineData("/api/auth/logout", true)]
    [InlineData("/api/auth/password", true)]
    [InlineData("/api/auth/state", false)]
    [InlineData("/api/auth/setup", false)]
    [InlineData("/api/auth/login", false)]
    [InlineData("/healthz", false)]
    [InlineData("/", false)]
    [InlineData("/index.html", false)]
    [InlineData("/assets/app.js", false)]
    [InlineData("/connect/token", false)]
    [InlineData("/api/remote/hello", false)]
    [InlineData("/api/hubs/remote", false)]
    [InlineData("/api/pair", false)]
    public void RequiresSignIn_protects_the_ui_api_by_default(string path, bool required) =>
        Surface.RequiresSignIn(path).Should().Be(required);
}
