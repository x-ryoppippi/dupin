//! Host services that the extension broker exposes to guests: reads and listings inside
//! user-picked file scopes, and user-approved executable profiles. Both act with the user's
//! native authority, so the broker checks grants before calling them. Nothing here prompts,
//! stores grants or talks to a guest.

pub mod exec;
pub mod files;

fn display_path(path: &std::path::Path) -> String {
    if let Some(home) = std::env::var_os("HOME").filter(|home| !home.is_empty())
        && let Ok(relative) = path.strip_prefix(&home)
    {
        return if relative.as_os_str().is_empty() {
            "~".into()
        } else {
            format!("~/{}", relative.display())
        };
    }
    path.display().to_string()
}
