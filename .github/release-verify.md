
## Verify

- `Koppel-Setup.exe` is not code signed yet, so Windows SmartScreen warns about it. Check it against `SHA256SUMS`.
- `SHA256SUMS` is signed with cosign, keyless, by this repository's release workflow. Check the signature, then the files:

  ```sh
  cosign verify-blob --bundle SHA256SUMS.sigstore.json \
    --certificate-identity-regexp '^https://github.com/matumi-software/koppel/.github/workflows/release.yml@refs/tags/v' \
    --certificate-oidc-issuer https://token.actions.githubusercontent.com SHA256SUMS
  sha256sum -c SHA256SUMS --ignore-missing
  ```

- The image is signed the same way:

  ```sh
  cosign verify matumisoftware/koppel:@VERSION@ \
    --certificate-identity-regexp '^https://github.com/matumi-software/koppel/.github/workflows/release.yml@refs/tags/v' \
    --certificate-oidc-issuer https://token.actions.githubusercontent.com
  ```
