//! Secure token storage via the OS credential store.
//!
//! Windows: Credential Manager (built-in, no extra deps).
//! Linux: Secret Service over D-Bus (needs `libsecret` / `gnome-keyring` / `kwallet`).
//!
//! The token is stored under service "think", user "github_token".

const SERVICE: &str = "think";
const USER: &str = "github_token";

/// Store the GitHub token in the OS keyring.
pub fn save_token(token: &str) -> Result<(), String> {
    let entry = keyring::Entry::new(SERVICE, USER).map_err(|e| e.to_string())?;
    entry.set_password(token).map_err(|e| e.to_string())
}

/// Read the GitHub token from the OS keyring.
pub fn load_token() -> Result<String, String> {
    let entry = keyring::Entry::new(SERVICE, USER).map_err(|e| e.to_string())?;
    entry.get_password().map_err(|e| e.to_string())
}

/// Remove the GitHub token from the OS keyring.
pub fn delete_token() -> Result<(), String> {
    let entry = keyring::Entry::new(SERVICE, USER).map_err(|e| e.to_string())?;
    entry.delete_credential().map_err(|e| e.to_string())
}

/// Check whether a token is already stored.
pub fn has_token() -> bool {
    load_token().is_ok()
}
