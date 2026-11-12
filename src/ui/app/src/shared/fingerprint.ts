/** A fingerprint as hex digits only, in lower case: the server accepts colons and spaces and stores its own spelling. */
export const hexOf = (fingerprint: string | null | undefined) => (fingerprint ?? '').replace(/[^0-9a-f]/gi, '').toLowerCase();

/** The first eight pairs, in the colon-separated upper case form the server's Clients page shows. */
export const shortFingerprint = (fingerprint: string) => {
  const pairs = hexOf(fingerprint).toUpperCase().match(/.{2}/g) ?? [];
  return `${pairs.slice(0, 8).join(':')}…`;
};
