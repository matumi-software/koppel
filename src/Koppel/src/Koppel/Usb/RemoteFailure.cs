using System.Net.WebSockets;
using Koppel.Core.Usb;
using Skakel.Client;

namespace Koppel.Usb;

/// <summary>Turns what can go wrong talking to a server into a sentence a person can act on.</summary>
internal static class RemoteFailure
{
    public static string Describe(Exception ex) => ex switch
    {
        // Skakel's fixed wording; the detail is in the log.
        SkakelException skakel => skakel.Failure.Message,
        TimeoutException or UsbHostException => ex.Message,
        WebSocketException or IOException or EndOfStreamException => SkakelFailure.Of(SkakelFailureKind.ConnectionLost).Message,
        _ => ex.Message,
    };
}
