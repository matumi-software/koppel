/** The words to show for a failed request: the server's own sentence when it gave one. */
export const reasonOf = (error: unknown, fallback: string) => (error instanceof Error ? error.message : fallback);

/** A sentence that stands alone starts with a capital: the same words after "Not saved: " do not. */
export const standalone = (text: string) => text.charAt(0).toUpperCase() + text.slice(1);
