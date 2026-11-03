using System.Net.Sockets;
using System.Security.Authentication;
using System.Text.Json;
using Koppel.Core.Auth;
using Koppel.Core.Usb;

namespace Koppel.Usb;

/// <summary>Turns what can go wrong talking to a server into a sentence a person can act on.</summary>
internal static class RemoteFailure
{
    public static string Describe(Exception ex) => ex switch
    {
        TimeoutException or TokenRequestException or UsbHostException => ex.Message,
        AuthenticationException => "The server's certificate does not match the saved fingerprint, or the secure connection could not be set up.",
        HttpRequestException { InnerException: AuthenticationException } => Describe(new AuthenticationException()),
        HttpRequestException http => $"Could not reach the server: {http.InnerException?.Message ?? http.Message}",
        JsonException => "The server did not answer like a Koppel server.",
        SocketException socket => $"Could not connect to the server: {socket.Message}",
        IOException io => $"The connection to the server was lost: {io.Message}",
        _ => ex.Message,
    };

    /// <summary>
    /// True when nothing answered at the server's address: the connection was refused or timed out, there
    /// was no route, or the name no longer resolves. A server that answered, with a certificate that does
    /// not match or by refusing the credentials, is not unreachable.
    /// </summary>
    public static bool IsUnreachable(Exception ex)
    {
        for (var e = ex; e is not null; e = e.InnerException)
        {
            switch (e)
            {
                case AuthenticationException or TokenRequestException:
                    return false;
                case HttpRequestException { HttpRequestError: HttpRequestError.ConnectionError or HttpRequestError.NameResolutionError }:
                case SocketException
                {
                    SocketErrorCode: SocketError.ConnectionRefused or SocketError.TimedOut or SocketError.HostUnreachable
                        or SocketError.NetworkUnreachable or SocketError.HostDown or SocketError.HostNotFound or SocketError.NoData,
                }:
                    return true;
            }
        }

        return false;
    }
}
