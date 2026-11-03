using System.Buffers;
using System.Net;
using System.Text;
using Microsoft.AspNetCore.Connections;
using Microsoft.AspNetCore.Server.Kestrel.Core;

namespace Koppel.Web;

/// <summary>
/// Answers plain HTTP on an HTTPS port with a redirect to the same address over HTTPS, so
/// <c>http://host:5240</c> in a browser lands on the UI instead of an empty page. It looks at the
/// first byte before TLS does: a TLS handshake starts with 0x16 and goes on untouched; anything else
/// gets a fixed 301 and the connection is closed. The plain request never reaches the app.
/// </summary>
internal static class PlainHttpRedirect
{
    /// <summary>Long enough for a slow client, short enough that a silent connection does not sit here before TLS's own timeout.</summary>
    private static readonly TimeSpan ReadTimeout = TimeSpan.FromSeconds(10);

    /// <summary>A request head larger than this is not a browser asking for a page: the connection is closed.</summary>
    private const int MaxHead = 8 * 1024;

    private const byte TlsHandshake = 0x16;

    /// <summary>Adds the redirect to a listener. Call it before <c>UseHttps</c>, so it sees the bytes before TLS.</summary>
    public static ListenOptions UsePlainHttpRedirect(this ListenOptions listen)
    {
        listen.Use(next => connection => RedirectOrContinueAsync(connection, next));
        return listen;
    }

    private static async Task RedirectOrContinueAsync(ConnectionContext connection, ConnectionDelegate next)
    {
        var input = connection.Transport.Input;
        using var timeout = CancellationTokenSource.CreateLinkedTokenSource(connection.ConnectionClosed);
        timeout.CancelAfter(ReadTimeout);
        try
        {
            var result = await input.ReadAsync(timeout.Token);
            if (result.Buffer.IsEmpty)
            {
                input.AdvanceTo(result.Buffer.Start);
                return;
            }

            if (result.Buffer.FirstSpan[0] == TlsHandshake)
            {
                // Nothing consumed and nothing examined, so TLS reads the same bytes at once.
                input.AdvanceTo(result.Buffer.Start);
                await next(connection);
                return;
            }

            while (true)
            {
                var buffer = result.Buffer;
                if (Location(buffer, connection.LocalEndPoint) is { } location)
                {
                    input.AdvanceTo(buffer.End);
                    await connection.Transport.Output.WriteAsync(Encoding.ASCII.GetBytes(
                        $"HTTP/1.1 301 Moved Permanently\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"), timeout.Token);
                    return;
                }

                if (buffer.Length >= MaxHead || result.IsCompleted || EndOfHead(buffer) is not null)
                {
                    // Too big, cut short, or a head that is not an HTTP request: just close.
                    input.AdvanceTo(buffer.End);
                    return;
                }

                input.AdvanceTo(buffer.Start, buffer.End);
                result = await input.ReadAsync(timeout.Token);
            }
        }
        catch (OperationCanceledException)
        {
            // The client said nothing, or not enough, in time.
        }
    }

    /// <summary>
    /// Where a complete plain HTTP request head should go over HTTPS, or null when the head is not
    /// complete yet or is not an HTTP request. The host comes from the Host header when it is a plain
    /// host name or address with an optional port, otherwise from the address the connection arrived
    /// on, so a request cannot put anything else into the Location header.
    /// </summary>
    internal static string? Location(ReadOnlySequence<byte> buffer, EndPoint? local)
    {
        if (EndOfHead(buffer) is not { } end)
        {
            return null;
        }

        var lines = Encoding.ASCII.GetString(buffer.Slice(0, end)).Split("\r\n");
        var requestLine = lines[0].Split(' ');
        if (requestLine.Length != 3 || !requestLine[2].StartsWith("HTTP/1.", StringComparison.Ordinal) || !requestLine[1].StartsWith('/'))
        {
            return null;
        }

        var host = lines.Skip(1)
            .Where(line => line.StartsWith("Host:", StringComparison.OrdinalIgnoreCase))
            .Select(line => line["Host:".Length..].Trim())
            .FirstOrDefault(IsPlainHost)
            ?? local?.ToString();
        return host is null ? null : $"https://{host}{requestLine[1]}";
    }

    private static long? EndOfHead(ReadOnlySequence<byte> buffer) =>
        new SequenceReader<byte>(buffer) is var reader && reader.TryReadTo(out ReadOnlySequence<byte> head, "\r\n\r\n"u8, advancePastDelimiter: false)
            ? head.Length
            : null;

    private static bool IsPlainHost(string host) =>
        host.Length is > 0 and <= 255 && host.All(c => char.IsAsciiLetterOrDigit(c) || c is '.' or '-' or ':' or '[' or ']');
}
