using System.Net;

namespace Koppel.Web;

/// <summary>
/// <c>Koppel --healthcheck</c>: the Docker image's health check. It sends GET /healthz to the UI port
/// and exits 0 on 200, 1 otherwise. The UI port is HTTPS, and the runtime image has no curl or openssl
/// for a shell script to speak it with.
/// </summary>
/// <remarks>
/// It asks where the UI is configured to listen (<c>Ports:Address</c> and <c>Ports:Ui</c>, read as
/// the host reads them), then 127.0.0.1, so a container bound to one LAN address is not unhealthy
/// forever just because nothing listens on loopback. Any address is reached through loopback.
/// </remarks>
internal static class HealthProbe
{
    public const string Flag = "--healthcheck";

    public static bool Requested(string[] args) => args.Contains(Flag, StringComparer.OrdinalIgnoreCase);

    public static async Task<int> RunAsync(IPAddress? address, int port)
    {
        using var client = new HttpClient(new SocketsHttpHandler
        {
#pragma warning disable CA5359 // Any certificate will do: the probe sends nothing secret and only asks whether Koppel answers.
            SslOptions = { RemoteCertificateValidationCallback = (_, _, _, _) => true },
#pragma warning restore CA5359
        })
        {
            Timeout = TimeSpan.FromSeconds(5),
        };

        var configured = address is null || address.Equals(IPAddress.Any) || address.Equals(IPAddress.IPv6Any) ? IPAddress.Loopback : address;
        foreach (var host in new[] { configured, IPAddress.Loopback }.Distinct())
        {
            try
            {
                using var response = await client.GetAsync(new Uri($"https://{new IPEndPoint(host, port)}/healthz"));
                return response.StatusCode == HttpStatusCode.OK ? 0 : 1;
            }
            catch (HttpRequestException)
            {
                // Nothing answered there: try the next address.
            }
            catch (TaskCanceledException)
            {
                // Something accepted the connection and never answered.
                return 1;
            }
        }

        return 1;
    }
}
