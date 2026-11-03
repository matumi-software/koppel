using System.Security.Claims;
using System.Security.Cryptography;
using Microsoft.AspNetCore.Http;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Options;
using Microsoft.Extensions.Primitives;
using Microsoft.IdentityModel.Tokens;
using OpenIddict.Abstractions;
using OpenIddict.Server;
using OpenIddict.Validation;
using OpenIddict.Validation.AspNetCore;
using static OpenIddict.Abstractions.OpenIddictConstants;
using static OpenIddict.Server.OpenIddictServerEvents;

namespace Koppel.Core.Auth;

/// <summary>Checks a client ID and secret against wherever the server keeps its registered clients.</summary>
public interface IClientCredentialValidator
{
    /// <summary>
    /// The generation of the client's credentials that the secret was verified against, or null when the
    /// client is unknown, revoked or the secret is wrong. The token is issued for exactly this generation,
    /// so a secret that is rotated in between never yields a token for the newer one. A rotated secret is
    /// a new generation, so tokens issued before the rotation stop working.
    /// </summary>
    ValueTask<int?> ValidateAsync(string clientId, string clientSecret, CancellationToken cancellationToken);
}

/// <summary>
/// Supplies the keys the server signs and checks access tokens with. They must survive restarts. After a
/// rotation the key it replaced stays among <see cref="ValidationKeys"/> until every token it signed has
/// expired, so a rotation logs nobody out.
/// </summary>
public interface ISigningKeySource
{
    /// <summary>The key new tokens are signed with. It needs a key id, which goes in each token's header.</summary>
    SecurityKey SigningKey { get; }

    /// <summary>Every key a token that is still valid may have been signed with: the signing key and the ones it replaced.</summary>
    IReadOnlyList<SecurityKey> ValidationKeys { get; }

    /// <summary>Fires when the keys change, so the token server and the validation pick them up.</summary>
    IChangeToken GetChangeToken();
}

/// <summary>
/// OAuth 2.0 client-credentials server (OpenIddict in degraded mode, so clients live in our own
/// encrypted file rather than a database) plus bearer-token validation for the server's own APIs.
/// </summary>
public static class KoppelAuthServer
{
    /// <summary>The token endpoint path.</summary>
    public const string TokenPath = "/connect/token";

    /// <summary>The claim that carries the client's credential generation (see <see cref="IClientCredentialValidator.ValidateAsync"/>).</summary>
    public const string GenerationClaim = "koppel_generation";

    /// <summary>How long an access token lasts. A replaced signing key is kept for validation at least this long.</summary>
    public static readonly TimeSpan AccessTokenLifetime = TimeSpan.FromMinutes(30);

    /// <summary>The authorization policy for endpoints only registered clients may call.</summary>
    public const string ClientPolicy = "koppel-client";

    /// <summary>The bearer authentication scheme for those endpoints.</summary>
    public const string Scheme = OpenIddictValidationAspNetCoreDefaults.AuthenticationScheme;

    /// <summary>
    /// Registers the token endpoint and bearer validation. The app supplies an
    /// <see cref="IClientCredentialValidator"/> and an <see cref="ISigningKeySource"/>.
    /// Token requests must arrive over HTTPS.
    /// </summary>
    public static IServiceCollection AddKoppelAuthServer(this IServiceCollection services)
    {
        services.AddOpenIddict()
            .AddServer(options =>
            {
                options.EnableDegradedMode();
                options.SetTokenEndpointUris(TokenPath);
                options.AllowClientCredentialsFlow();
                options.SetAccessTokenLifetime(AccessTokenLifetime);

                // Only access tokens are issued, and they are signed, not encrypted. The encryption key
                // OpenIddict insists on is never used, so a random one per start is enough. It is
                // symmetric: AddEphemeralEncryptionKey makes a 2048-bit RSA key on the first request,
                // which took 8 to 14 seconds on a low-power thin client and timed out its first caller.
                options.DisableAccessTokenEncryption();
                options.AddEncryptionKey(new SymmetricSecurityKey(RandomNumberGenerator.GetBytes(32)));

                options.UseAspNetCore();

                options.AddEventHandler<ValidateTokenRequestContext>(builder =>
                    builder.UseScopedHandler<ValidateClientCredentials>());
                options.AddEventHandler<HandleTokenRequestContext>(builder =>
                    builder.UseScopedHandler<IssueClientToken>());
            })
            .AddValidation(options =>
            {
                options.UseLocalServer();
                options.UseAspNetCore();
            });

        // Only the current key signs. Validation also takes the keys it replaced, which are still live: the
        // local server integration copies the server's signing keys into the validation's configuration
        // first, and this adds the rest after it. Both option sets are built again when the keys change.
        services.AddOptions<OpenIddictServerOptions>()
            .Configure<ISigningKeySource>((options, keys) =>
                options.SigningCredentials.Add(new SigningCredentials(keys.SigningKey, SecurityAlgorithms.RsaSha256)));
        services.AddOptions<OpenIddictValidationOptions>()
            .Configure<ISigningKeySource>((options, keys) =>
            {
                if (options.Configuration is not { } configuration)
                {
                    return;
                }

                foreach (var key in keys.ValidationKeys.Where(key => configuration.SigningKeys.All(known => known.KeyId != key.KeyId)))
                {
                    configuration.SigningKeys.Add(key);
                }
            });
        services.AddSingleton<IOptionsChangeTokenSource<OpenIddictServerOptions>, SigningKeyChange<OpenIddictServerOptions>>();
        services.AddSingleton<IOptionsChangeTokenSource<OpenIddictValidationOptions>, SigningKeyChange<OpenIddictValidationOptions>>();

        services.AddAuthentication(Scheme);
        services.AddAuthorizationBuilder()
            .AddPolicy(ClientPolicy, policy => policy
                .AddAuthenticationSchemes(Scheme)
                .RequireAuthenticatedUser()
                // A bearer token sent over plain HTTP has already leaked; refuse it.
                .RequireAssertion(context => context.Resource is HttpContext { Request.IsHttps: true }));

        return services;
    }

    /// <summary>The client ID a validated bearer token was issued to.</summary>
    public static string? GetClientId(this ClaimsPrincipal principal)
    {
        ArgumentNullException.ThrowIfNull(principal);
        return principal.FindFirst(Claims.Subject)?.Value;
    }

    /// <summary>The credential generation a validated bearer token was issued for; null when it carries none.</summary>
    public static int? GetGeneration(this ClaimsPrincipal principal)
    {
        ArgumentNullException.ThrowIfNull(principal);
        return int.TryParse(principal.FindFirst(GenerationClaim)?.Value, System.Globalization.NumberStyles.None, System.Globalization.CultureInfo.InvariantCulture, out var generation) ? generation : null;
    }

    /// <summary>Tells the options system to build the token server's and the validation's options again when the keys change.</summary>
    private sealed class SigningKeyChange<TOptions>(ISigningKeySource keys) : IOptionsChangeTokenSource<TOptions>
    {
        public string Name => Options.DefaultName;

        public IChangeToken GetChangeToken() => keys.GetChangeToken();
    }

    /// <summary>The key under which validation hands the verified generation to the issuing handler.</summary>
    private const string VerifiedGeneration = "koppel:verified-generation";

    private sealed class IssueClientToken : IOpenIddictServerHandler<HandleTokenRequestContext>
    {
        public ValueTask HandleAsync(HandleTokenRequestContext context)
        {
            if (!context.Transaction.Properties.TryGetValue(VerifiedGeneration, out var verified) || verified is not int generation)
            {
                context.Reject(Errors.InvalidClient, "The client credentials were not verified.");
                return ValueTask.CompletedTask;
            }

            var identity = new ClaimsIdentity(TokenValidationParameters.DefaultAuthenticationType, Claims.Name, Claims.Role);
            identity.SetClaim(Claims.Subject, context.Request.ClientId);
            identity.SetClaim(GenerationClaim, generation.ToString(System.Globalization.CultureInfo.InvariantCulture));
            identity.SetDestinations(static _ => [Destinations.AccessToken]);
            context.SignIn(new ClaimsPrincipal(identity));
            return ValueTask.CompletedTask;
        }
    }

    private sealed class ValidateClientCredentials(IClientCredentialValidator validator) : IOpenIddictServerHandler<ValidateTokenRequestContext>
    {
        public async ValueTask HandleAsync(ValidateTokenRequestContext context)
        {
            if (!context.Request.IsClientCredentialsGrantType())
            {
                context.Reject(Errors.UnsupportedGrantType, "Only the client_credentials grant is supported.");
                return;
            }

            if (string.IsNullOrEmpty(context.ClientId) || string.IsNullOrEmpty(context.ClientSecret)
                || await validator.ValidateAsync(context.ClientId, context.ClientSecret, context.CancellationToken) is not { } generation)
            {
                context.Reject(Errors.InvalidClient, "The client ID or secret is wrong, or the client was revoked.");
                return;
            }

            context.Transaction.Properties[VerifiedGeneration] = generation;
        }
    }
}
