using System.Net;
using Koppel.Web;

namespace Koppel.Tests.Web;

public sealed class ListenAddressTests
{
    [Fact]
    public void Resolve_binds_to_any_address_unless_told_otherwise() =>
        ListenAddress.Resolve(null).Should().BeNull();

    [Fact]
    public void Resolve_uses_the_setting_over_the_default() =>
        ListenAddress.Resolve("127.0.0.1").Should().Be(IPAddress.Loopback);

    [Fact]
    public void Resolve_rejects_a_value_that_is_not_an_address_and_names_the_setting()
    {
        var resolve = () => ListenAddress.Resolve("localhost");

        resolve.Should().Throw<InvalidOperationException>().Which.Message.Should().Contain("Ports:Address");
    }
}