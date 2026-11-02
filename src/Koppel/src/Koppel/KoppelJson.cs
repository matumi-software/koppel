using System.Text.Json;
using System.Text.Json.Serialization;

namespace Koppel;

/// <summary>JSON settings shared by the config files, the REST API and the hub: camelCase, enums as strings.</summary>
internal static class KoppelJson
{
    public static JsonSerializerOptions Files { get; } = new(JsonSerializerDefaults.Web)
    {
        WriteIndented = true,
        Converters = { new JsonStringEnumConverter(JsonNamingPolicy.CamelCase) },
    };

    public static void Configure(JsonSerializerOptions options)
    {
        ArgumentNullException.ThrowIfNull(options);
        options.Converters.Add(new JsonStringEnumConverter(JsonNamingPolicy.CamelCase));
    }
}
