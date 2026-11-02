using System.Security.Claims;
using System.Security.Cryptography;
using Microsoft.AspNetCore.Http;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.IdentityModel.Tokens;
using OpenIddict.Abstractions;
using OpenIddict.Server;
using OpenIddict.Validation.AspNetCore;
using static OpenIddict.Abstractions.OpenIddictConstants;
using static OpenIddict.Server.OpenIddictServerEvents;

namespace Koppel.Core.Auth;

/// <summary>Checks a client ID and secret against wherever the server keeps its registered clients.</summary>
public interface IClientCredentialValidator
{
    ValueTask<bool> ValidateAsync(string clientId, string clientSecret, CancellationToken cancellationToken);
}

/// <summary>Supplies the key the server signs access tokens with. It must survive restarts.</summary>
public interface ISigningKeySource
{
    SecurityKey SigningKey { get; }
}

/// <summary>
/// OAuth 2.0 client-credentials server (OpenIddict in degraded mode, so clients live in our own
/// encrypted file rather than a database) plus bearer-token validation for the server's own APIs.
/// </summary>
public static class KoppelAuthServer
{
    /// <summary>The token endpoint path.</summary>
    public const string TokenPath = "/connect/token";

    /// <summary>The authorization policy for endpoints only registered clients may call.</summary>
    public const string ClientPolicy = "koppel-client";

    /// <summary>The bearer authentication scheme for those endpoints.</summary>
    public const string Scheme = OpenIddictValidationAspNetCoreDefaults.AuthenticationScheme;

    /// <summary>
    /// Registers the token endpoint and bearer validation. The app supplies an
    /// <see cref="IClientCredentialValidator"/> and an <see cref="ISigningKeySource"/>.
    /// Token requests must arrive over HTTPS.
    /// </summary>
    public static IServiceCollection AddKoppelAuthServer(this IServiceCollection services, TimeSpan? accessTokenLifetime = null)
    {
        services.AddOpenIddict()
            .AddServer(options =>
            {
                options.EnableDegradedMode();
                options.SetTokenEndpointUris(TokenPath);
                options.AllowClientCredentialsFlow();
                options.SetAccessTokenLifetime(accessTokenLifetime ?? TimeSpan.FromMinutes(30));

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
                    builder.UseInlineHandler(IssueClientToken));
            })
            .AddValidation(options =>
            {
                options.UseLocalServer();
                options.UseAspNetCore();
            });

        services.AddOptions<OpenIddictServerOptions>()
            .Configure<ISigningKeySource>((options, keys) =>
                options.SigningCredentials.Add(new SigningCredentials(keys.SigningKey, SecurityAlgorithms.RsaSha256)));

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

    private static ValueTask IssueClientToken(HandleTokenRequestContext context)
    {
        var identity = new ClaimsIdentity(TokenValidationParameters.DefaultAuthenticationType, Claims.Name, Claims.Role);
        identity.SetClaim(Claims.Subject, context.Request.ClientId);
        identity.SetDestinations(static _ => [Destinations.AccessToken]);
        context.SignIn(new ClaimsPrincipal(identity));
        return ValueTask.CompletedTask;
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
                || !await validator.ValidateAsync(context.ClientId, context.ClientSecret, context.CancellationToken))
            {
                context.Reject(Errors.InvalidClient, "The client ID or secret is wrong, or the client was revoked.");
            }
        }
    }
}
