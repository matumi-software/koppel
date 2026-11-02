using System.Net;
using System.Net.Sockets;
using System.Runtime.InteropServices;

namespace Koppel.Core.Usb.Windows;

/// <summary>One IPv4 TCP connection as Windows lists it, with the process that owns it.</summary>
public readonly record struct TcpRow(IPAddress LocalAddress, int LocalPort, IPAddress RemoteAddress, int RemotePort, uint OwningPid);

/// <summary>The machine's TCP connections. A fake stands in for it in tests.</summary>
public interface ITcpTable
{
    IReadOnlyList<TcpRow> Rows();
}

/// <summary>
/// Decides whether a connection to the loopback gateway came from the usbip-win2 driver. The driver
/// connects from the kernel, so Windows lists the connection under the System process (PID 4);
/// anything a user-mode process opens is listed under that process's own PID. The gateway gives the
/// client's access to the server to whoever reaches it, so on a machine with several users or
/// services it only accepts the kernel.
/// </summary>
public sealed class KernelPeerCheck(ITcpTable table, int attempts = 3, TimeSpan? retryDelay = null)
{
    /// <summary>The System process, the owner of connections the kernel itself makes.</summary>
    public const uint KernelPid = 4;

    private readonly TimeSpan _retryDelay = retryDelay ?? TimeSpan.FromMilliseconds(50);

    /// <summary>
    /// True when <paramref name="peer"/> (the accepted connection's remote end) is a connection to
    /// <paramref name="gateway"/> that the kernel owns. The row is looked up from the client side: its
    /// local end is the peer and its remote end is the gateway.
    /// </summary>
    public async Task<bool> IsKernelAsync(IPEndPoint peer, IPEndPoint gateway, CancellationToken cancellationToken)
    {
        for (var attempt = 0; attempt < attempts; attempt++)
        {
            if (Owner(table.Rows(), peer, gateway) is { } pid)
            {
                return pid == KernelPid;
            }

            // The table can trail the accept by a moment.
            await Task.Delay(_retryDelay, cancellationToken);
        }

        return false;
    }

    /// <summary>The owner of the connection from <paramref name="peer"/> to <paramref name="gateway"/>, or null when it is not listed.</summary>
    public static uint? Owner(IReadOnlyList<TcpRow> rows, IPEndPoint peer, IPEndPoint gateway)
    {
        foreach (var row in rows)
        {
            if (row.LocalPort == peer.Port && row.RemotePort == gateway.Port
                && row.LocalAddress.Equals(peer.Address) && row.RemoteAddress.Equals(gateway.Address))
            {
                return row.OwningPid;
            }
        }

        return null;
    }
}

/// <summary>Windows' <c>GetExtendedTcpTable</c>, the same list <c>netstat -ano</c> prints.</summary>
public sealed partial class WindowsTcpTable : ITcpTable
{
    private const int AfInet = 2;
    private const int TcpTableOwnerPidAll = 5;
    private const int InsufficientBuffer = 122;

    public IReadOnlyList<TcpRow> Rows()
    {
        var size = 0;
        var buffer = IntPtr.Zero;
        try
        {
            // The table can grow between the size query and the read, so ask again if it did.
            for (var attempt = 0; attempt < 5; attempt++)
            {
                var result = GetExtendedTcpTable(buffer, ref size, false, AfInet, TcpTableOwnerPidAll, 0);
                if (result == 0)
                {
                    return Parse(buffer);
                }

                if (result != InsufficientBuffer)
                {
                    throw new UsbHostException($"Could not read the TCP table (error {result}).");
                }

                Marshal.FreeHGlobal(buffer);
                buffer = Marshal.AllocHGlobal(size);
            }

            throw new UsbHostException("The TCP table kept changing while it was read.");
        }
        finally
        {
            if (buffer != IntPtr.Zero)
            {
                Marshal.FreeHGlobal(buffer);
            }
        }
    }

    /// <summary>MIB_TCPTABLE_OWNER_PID: a count, then rows of state, local addr, local port, remote addr, remote port, pid.</summary>
    private static List<TcpRow> Parse(IntPtr table)
    {
        var count = Marshal.ReadInt32(table);
        var rows = new List<TcpRow>(count);
        for (var i = 0; i < count; i++)
        {
            var row = table + 4 + (i * 24);
            rows.Add(new TcpRow(
                new IPAddress((uint)Marshal.ReadInt32(row, 4)),
                Port(Marshal.ReadInt32(row, 8)),
                new IPAddress((uint)Marshal.ReadInt32(row, 12)),
                Port(Marshal.ReadInt32(row, 16)),
                (uint)Marshal.ReadInt32(row, 20)));
        }

        return rows;
    }

    /// <summary>The port sits in the low 16 bits in network byte order.</summary>
    private static int Port(int raw) => ((raw & 0xff) << 8) | ((raw >> 8) & 0xff);

    [LibraryImport("iphlpapi.dll")]
    private static partial int GetExtendedTcpTable(IntPtr table, ref int size, [MarshalAs(UnmanagedType.Bool)] bool sort, int family, int tableClass, int reserved);
}
