/** A fingerprint as hex digits only, in lower case: the server accepts colons and spaces and stores its own spelling. */
export const hexOf = (fingerprint: string | null | undefined) => (fingerprint ?? '').replace(/[^0-9a-f]/gi, '').toLowerCase();
