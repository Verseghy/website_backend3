//! Boot-time behaviour: the app clock every context carries, the admin UI
//! directory check, the configuration files, the database-target guard on
//! loaded configurations, and the session settings every connection gets.

mod admin_dist;
mod clock;
mod config_files;
mod database_guard;
mod session_settings;
