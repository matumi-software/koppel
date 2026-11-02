using System.Net;
using System.Net.Http.Headers;
using System.Net.Http.Json;
using System.Net.Security;
using System.Net.Sockets;
using System.Text;
using System.Text.Json;
using Koppel.Core.Security;
using Microsoft.AspNetCore.SignalR.Client;
using Microsoft.Extensions.DependencyInjection;

namespace Koppel.Tests.Usb;

/// <summary>
/// Plays a remote client against a server host, below the level of Koppel's own client: it takes a
/// token, opens the USB port and the remote hub, and trusts any certificate, so a test can watch what
/// the server does to each connection.
/// </summary>
internal static class RemoteAccess
{
    public static string Fingerprint(KoppelFactory server) =>
        server.Services.GetRequiredService<Koppel.Server.ServerSecretsStore>().Fingerprint;

    public static bool Pinned(KoppelFactory server, System.Security.Cryptography.X509Certificates.X509Certificate? certificate) =>
        ServerCertificate.Matches(certificate as System.Security.Cryptography.X509Certificates.X509Certificate2, Fingerprint(server));

    public static HttpClient Https(KoppelFactory server) =>
        new(ServerCertificate.CreatePinnedHandler(Fingerprint(server))) { BaseAddress = new Uri($"https://localhost:{server.HttpsPort}") };

    public static async Task<(string ClientId, string Secret)> CreateClientAsync(KoppelFactory server, string name, CancellationToken ct)
    {
        using var ui = server.CreateUiClient();
        using var response = await ui.PostAsJsonAsync("/api/server/clients", new { name }, ct);
        response.EnsureSuccessStatusCode();
        var body = await response.Content.ReadFromJsonAsync<JsonElement>(ct);
        return (body.GetProperty("clientId").GetString()!, body.GetProperty("clientSecret").GetString()!);
    }

    public static async Task<HttpResponseMessage> RequestTokenAsync(KoppelFactory server, string clientId, string secret, CancellationToken ct)
    {
        using var https = Https(server);
        return await https.PostAsync("/connect/token", new FormUrlEncodedContent(new Dictionary<string, string>
        {
            ["grant_type"] = "client_credentials",
            ["client_id"] = clientId,
            ["client_secret"] = secret,
        }), ct);
    }

    public static async Task<string> TokenAsync(KoppelFactory server, string clientId, string secret, CancellationToken ct)
    {
        using var response = await RequestTokenAsync(server, clientId, secret, ct);
        response.EnsureSuccessStatusCode();
        return (await response.Content.ReadFromJsonAsync<JsonElement>(ct)).GetProperty("access_token").GetString()!;
    }

    public static async Task<HttpStatusCode> HelloAsync(KoppelFactory server, string token, CancellationToken ct)
    {
        using var https = Https(server);
        using var request = new HttpRequestMessage(HttpMethod.Get, "/api/remote/hello");
        request.Headers.Authorization = new AuthenticationHeaderValue("Bearer", token);
        using var response = await https.SendAsync(request, ct);
        return response.StatusCode;
    }

    /// <summary>Opens the USB port, speaks TLS and sends the preface. Returns the stream and the server's one-line answer.</summary>
    public static async Task<(SslStream Tls, string? Answer)> OpenUsbAsync(KoppelFactory server, string token, CancellationToken ct, int? port = null)
    {
        var tcp = new TcpClient();
        await tcp.ConnectAsync("localhost", port ?? server.UsbPort, ct);
        var tls = new SslStream(tcp.GetStream(), leaveInnerStreamOpen: false, (_, certificate, _, _) => Pinned(server, certificate));
        await tls.AuthenticateAsClientAsync("localhost");
        await tls.WriteAsync(Encoding.ASCII.GetBytes($"KOPPEL/1 {token}\n"), ct);
        var answer = await Koppel.Usb.UsbIpTlsServer.ReadLineAsync(tls, ct);
        return (tls, answer);
    }

    /// <summary>Imports <paramref name="busId"/> on a session that was answered OK. The stream then carries USB traffic.</summary>
    public static async Task ImportAsync(SslStream tls, string busId, CancellationToken ct)
    {
        await tls.WriteAsync(Koppel.Core.Usb.UsbIpProtocol.ImportRequest(busId), ct);
        var header = await Koppel.Core.Usb.UsbIpProtocol.ReadHeaderAsync(tls, ct);
        header.Status.Should().Be(Koppel.Core.Usb.UsbIpProtocol.StatusOk);
        await Koppel.Core.Usb.UsbIpProtocol.ReadImportedDeviceAsync(tls, ct);
    }

    /// <summary>Opens the remote hub as a client with this token. <c>Closed</c> fires when the server ends the link.</summary>
    public static async Task<(HubConnection Hub, Task Closed, Task<JsonElement> First)> ConnectHubAsync(KoppelFactory server, string token, CancellationToken ct)
    {
        var hub = new HubConnectionBuilder()
            .WithUrl($"https://localhost:{server.HttpsPort}/api/hubs/remote", options =>
            {
                options.AccessTokenProvider = () => Task.FromResult<string?>(token);
                options.HttpMessageHandlerFactory = _ => ServerCertificate.CreatePinnedHandler(Fingerprint(server));
                options.WebSocketConfiguration = ws => ws.RemoteCertificateValidationCallback = (_, certificate, _, _) => Pinned(server, certificate);
            })
            .Build();
        var closed = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        hub.Closed += _ =>
        {
            closed.TrySetResult();
            return Task.CompletedTask;
        };
        var first = new TaskCompletionSource<JsonElement>(TaskCreationOptions.RunContinuationsAsynchronously);
        hub.On<JsonElement>("DevicesChanged", devices => first.TrySetResult(devices));
        await hub.StartAsync(ct);
        return (hub, closed.Task, first.Task);
    }

    /// <summary>True when the server will not keep a hub link for this token: the start fails, or the link is closed at once.</summary>
    public static async Task<bool> HubRefusesAsync(KoppelFactory server, string token, CancellationToken ct)
    {
        try
        {
            var (hub, closed, _) = await ConnectHubAsync(server, token, ct);
            await using var link = hub;
            await closed.WaitAsync(TimeSpan.FromSeconds(10), ct);
            return true;
        }
        catch (Exception ex) when (ex is HttpRequestException or InvalidOperationException or IOException)
        {
            return true;
        }
        catch (TimeoutException)
        {
            return false;
        }
    }

    /// <summary>True when the stream ends (or breaks) within the time. A session the server ended reads as that.</summary>
    public static async Task<bool> EndsWithinAsync(Stream stream, TimeSpan time, CancellationToken ct)
    {
        using var limit = CancellationTokenSource.CreateLinkedTokenSource(ct);
        limit.CancelAfter(time);
        var buffer = new byte[64];
        try
        {
            while (await stream.ReadAsync(buffer, limit.Token) > 0)
            {
                // Drain until the end.
            }

            return true;
        }
        catch (Exception ex) when (ex is IOException or SocketException or ObjectDisposedException)
        {
            return true;
        }
        catch (OperationCanceledException) when (!ct.IsCancellationRequested)
        {
            return false;
        }
    }
}
