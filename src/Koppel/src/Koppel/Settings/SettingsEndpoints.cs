using Koppel.Core.Activity;

namespace Koppel.Settings;

internal static class SettingsEndpoints
{
    /// <summary>
    /// GET returns the settings as stored. PATCH writes a change and answers 202 with no body:
    /// the confirmed settings reach the UI over SignalR (<c>SettingsChanged</c>), never in the response.
    /// </summary>
    public static IEndpointRouteBuilder MapSettingsEndpoints(this IEndpointRouteBuilder app)
    {
        var group = app.MapGroup("/api/settings");

        group.MapGet("/", (SettingsStore store) => TypedResults.Ok(store.Current));

        group.MapPatch("/", async Task<IResult> (SettingsPatch patch, SettingsStore store, ReservedPorts reserved, IActivityRecorder activity, CancellationToken cancellationToken) =>
        {
            var errors = patch.Validate(reserved);
            if (errors.Count > 0)
            {
                return TypedResults.ValidationProblem(errors);
            }

            // A change that leaves everything as it is writes no activity, so repeating it cannot flood the log.
            var summary = patch.Describe(store.Current);
            await store.ApplyAsync(patch, cancellationToken);
            if (summary.Length > 0)
            {
                await activity.RecordAsync(new EntityRef("machine", Environment.MachineName, "machine"), ActivityReason.Reconfigured, ActivityOutcome.Succeeded, summary);
            }

            return TypedResults.Accepted((string?)null);
        });

        return app;
    }
}
