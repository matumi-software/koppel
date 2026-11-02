using System.Net;
using System.Net.Sockets;
using Koppel.Core.Usb.Windows;

namespace Koppel.Core.Usb.Tests;

public sealed class KernelPeerCheckTests
{
    private static readonly IPEndPoint Gateway = new(IPAddress.Loopback, 3242);
    private static readonly IPEndPoint Peer = new(IPAddress.Loopback, 50000);

    private static TcpRow Row(int localPort, int remotePort, uint pid) =>
        new(IPAddress.Loopback, localPort, IPAddress.Loopback, remotePort, pid);

    private sealed class FakeTable(params Func<IReadOnlyList<TcpRow>>[] snapshots) : ITcpTable
    {
        private int _reads;

        public int Reads => _reads;

        public IReadOnlyList<TcpRow> Rows() => snapshots[Math.Min(_reads++, snapshots.Length - 1)]();
    }

    private static KernelPeerCheck Check(ITcpTable table) => new(table, attempts: 3, retryDelay: TimeSpan.Zero);

    [Fact]
    public async Task Connection_owned_by_the_kernel_is_allowed()
    {
        var table = new FakeTable(() => [Row(50000, 3242, KernelPeerCheck.KernelPid)]);

        (await Check(table).IsKernelAsync(Peer, Gateway, TestContext.Current.CancellationToken)).Should().BeTrue();
    }

    [Fact]
    public async Task Connection_owned_by_a_user_process_is_refused()
    {
        var table = new FakeTable(() => [Row(50000, 3242, 4321)]);

        (await Check(table).IsKernelAsync(Peer, Gateway, TestContext.Current.CancellationToken)).Should().BeFalse();
    }

    [Fact]
    public async Task Only_the_row_for_this_connection_counts()
    {
        // The kernel owns another connection to the gateway; this one is a user process's.
        var table = new FakeTable(() => [Row(50001, 3242, KernelPeerCheck.KernelPid), Row(50000, 3242, 4321), Row(50000, 9999, KernelPeerCheck.KernelPid)]);

        (await Check(table).IsKernelAsync(Peer, Gateway, TestContext.Current.CancellationToken)).Should().BeFalse();
    }

    [Fact]
    public async Task Connection_missing_from_the_table_is_refused_after_a_few_tries()
    {
        var table = new FakeTable(() => []);

        (await Check(table).IsKernelAsync(Peer, Gateway, TestContext.Current.CancellationToken)).Should().BeFalse();
        table.Reads.Should().Be(3);
    }

    [Fact]
    public async Task Connection_that_appears_a_moment_late_is_found()
    {
        var table = new FakeTable(() => [], () => [Row(50000, 3242, KernelPeerCheck.KernelPid)]);

        (await Check(table).IsKernelAsync(Peer, Gateway, TestContext.Current.CancellationToken)).Should().BeTrue();
    }

    [Fact]
    public async Task Windows_tcp_table_lists_a_loopback_connection_under_its_owner()
    {
        if (!OperatingSystem.IsWindows())
        {
            return;
        }

        using var listener = new TcpListener(IPAddress.Loopback, 0);
        listener.Start();
        var gateway = (IPEndPoint)listener.LocalEndpoint;
        using var client = new TcpClient();
        await client.ConnectAsync(gateway, TestContext.Current.CancellationToken);
        using var accepted = await listener.AcceptTcpClientAsync(TestContext.Current.CancellationToken);
        var peer = (IPEndPoint)accepted.Client.RemoteEndPoint!;

        var owner = KernelPeerCheck.Owner(new WindowsTcpTable().Rows(), peer, gateway);

        owner.Should().Be((uint)Environment.ProcessId);
        (await new KernelPeerCheck(new WindowsTcpTable()).IsKernelAsync(peer, gateway, TestContext.Current.CancellationToken)).Should().BeFalse();
    }
}
