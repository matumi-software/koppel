using System.Text.Json.Serialization;
using Koppel;
using Koppel.Hubs;
using Koppel.Settings;

var builder = WebApplication.CreateBuilder(args);

builder.Services.ConfigureHttpJsonOptions(options =>
    options.SerializerOptions.Converters.Add(new JsonStringEnumConverter(System.Text.Json.JsonNamingPolicy.CamelCase)));

builder.Services
    .AddSignalR()
    .AddJsonProtocol(options =>
        options.PayloadSerializerOptions.Converters.Add(new JsonStringEnumConverter(System.Text.Json.JsonNamingPolicy.CamelCase)));

builder.Services.AddSingleton(KoppelPaths.FromConfiguration(builder.Configuration, builder.Environment));
builder.Services.AddSingleton(TimeProvider.System);
builder.Services.AddSingleton<SettingsStore>();
builder.Services.AddHostedService(sp => sp.GetRequiredService<SettingsStore>());
builder.Services.AddHostedService<SettingsHubRelay>();

var app = builder.Build();

if (app.Environment.IsDevelopment())
{
    app.UseDeveloperExceptionPage();
}

// Static files before routing, so the SPA assets never hit an endpoint.
app.UseStaticFiles();

app.MapSettingsEndpoints();
app.MapHub<UiHub>("/api/hubs/ui");

app.MapFallbackToFile("index.html");

app.Run();
