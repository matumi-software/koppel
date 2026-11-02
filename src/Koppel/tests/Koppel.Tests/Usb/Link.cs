using System.Net.Http.Json;
using System.Text.Json;
using Microsoft.AspNetCore.SignalR.Client;

namespace Koppel.Tests.Usb;

/// <summary>Links a client host to a server host the way a person would in the UI, and waits on hub pushes.</summary>
internal static class Link
{
    /// <summary>Registers a client on the server, then gives the client its URL, fingerprint and credentials.</summary>
    public static async Task ConnectAsync(KoppelFactory serverHost, KoppelFactory clientHost, CancellationToken ct)
    {
        using var server = serverHost.CreateUiClient();
        var identity = await server.GetFromJsonAsync<JsonElement>("/api/server/identity", ct);
        using var created = await server.PostAsJsonAsync("/api/server/clients", new { name = "test-client" }, ct);
        var credentials = await created.Content.ReadFromJsonAsync<JsonElement>(ct);

        using var client = clientHost.CreateUiClient();
        (await client.PatchAsJsonAsync("/api/settings", new { role = "client" }, ct)).EnsureSuccessStatusCode();
        (await client.PutAsJsonAsync("/api/client/connection", new
        {
            serverUrl = $"https://localhost:{serverHost.HttpsPort}",
            serverFingerprint = identity.GetProperty("fingerprint").GetString(),
            clientId = credentials.GetProperty("clientId").GetString(),
            clientSecret = credentials.GetProperty("clientSecret").GetString(),
        }, ct)).EnsureSuccessStatusCode();
    }

    /// <summary>Waits for <paramref name="waiter"/>; on timeout, says what the last push was.</summary>
    public static async Task<JsonElement> WithLastAsync(Task<JsonElement> waiter, HubConnection hub, CancellationToken ct)
    {
        var last = "nothing";
        hub.On<JsonElement>("ClientDevicesChanged", value => last = value.GetRawText());
        try
        {
            return await waiter.WaitAsync(TimeSpan.FromSeconds(20), ct);
        }
        catch (TimeoutException)
        {
            throw new TimeoutException($"Last ClientDevicesChanged: {last}");
        }
    }

    /// <summary>Completes with the first push of <paramref name="method"/> that matches <paramref name="until"/>.</summary>
    public static Task<JsonElement> Waiter(HubConnection hub, string method, Func<JsonElement, bool> until)
    {
        var done = new TaskCompletionSource<JsonElement>(TaskCreationOptions.RunContinuationsAsynchronously);
        hub.On<JsonElement>(method, value =>
        {
            if (until(value))
            {
                done.TrySetResult(value);
            }
        });
        return done.Task;
    }
}
