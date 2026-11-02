/** The server refused the request. The message is ready to show after "Not saved: ". */
export class RejectedError extends Error {}

/** Reads the first validation message from a 400 problem response, or a generic one. */
export async function rejection(response: Response): Promise<RejectedError> {
  if (response.status === 400) {
    try {
      const problem = (await response.json()) as { errors?: Record<string, string[]> };
      const first = Object.values(problem.errors ?? {})[0]?.[0];
      if (first) return new RejectedError(first);
    } catch {
      // Not a problem document; fall through.
    }
  }
  return new RejectedError(`the server answered ${response.status}.`);
}

export async function getJson<T>(url: string): Promise<T> {
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`Could not load ${url} (${response.status}).`);
  }
  return (await response.json()) as T;
}

/**
 * Sends a change and resolves once the server accepted it (202). That is not a confirmation:
 * the confirmed value arrives over the hub.
 */
export async function sendChange(url: string, method: 'PATCH' | 'PUT' | 'DELETE', body?: unknown): Promise<void> {
  const response = await fetch(url, {
    method,
    headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (response.status !== 202) {
    throw await rejection(response);
  }
}
