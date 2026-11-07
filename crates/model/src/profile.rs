use serde::Deserialize;

/// The signed-in Spotify user, as `/me` describes them.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub display_name: Option<Box<str>>,
}
