/**
 * What the server accepts as the admin password, so a form can say so before it asks. The server
 * decides (`koppel-web/src/auth/password.rs`): these are its rules, worded as it words them, and
 * anything it still refuses is shown as it answers.
 */
export const minPasswordLength = 10;
export const maxPasswordLength = 128;

const obvious = ['koppel', 'password'];

/** Why a new password is not acceptable, or null. */
export function passwordProblem(password: string): string | null {
  if (password.length < minPasswordLength) return `Use at least ${minPasswordLength} characters.`;
  if (password.length > maxPasswordLength) return `Use at most ${maxPasswordLength} characters.`;
  if (obvious.includes(password.toLowerCase())) return 'That password is too easy to guess.';
  return null;
}

/** Why the repeated password does not match, or null. */
export const repeatProblem = (again: string, password: string): string | null => (again === password ? null : 'The two passwords differ.');

/** The line under a new password field before anything is wrong. */
export const passwordHelp = `At least ${minPasswordLength} characters.`;
