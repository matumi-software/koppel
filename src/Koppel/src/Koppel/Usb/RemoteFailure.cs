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
}
