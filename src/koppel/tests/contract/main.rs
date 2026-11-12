//! Black-box tests: each starts the real `koppel` program on a free port with a temporary config and
//! keys folder, and drives it over HTTPS as a browser or another Koppel machine does. See `support`.

mod auth;
mod client;
mod files;
mod first_run;
mod guards;
mod push;
mod server;
mod settings;
mod support;
