/**
 * The shortest admin password the server accepts, so a form can keep its action off a shorter one. The server
 * decides (`koppel-web/src/auth/password.rs`): anything else it refuses is shown as it answers.
 */
export const minPasswordLength = 10;
