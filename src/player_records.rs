use std::{
    collections::HashMap,
    fmt::Display,
    io::ErrorKind,
    ops::{Deref, DerefMut},
    path::PathBuf,
    sync::Arc,
};

use anyhow::Context;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Map;
use steamid_ng::SteamID;

use crate::{
    args::Args,
    settings::{ConfigFilesError, Settings},
};

// PlayerList

#[derive(Serialize, Deserialize)]
pub struct PlayerRecords {
    #[serde(skip)]
    path: PathBuf,
    pub records: HashMap<SteamID, PlayerRecord>,
}

impl PlayerRecords {
    /// Attempts to load the playerlist from the overriden (if provided in
    /// [Args]) or default location. If it cannot be found, then a new one
    /// is created at the location.
    ///
    /// # Panics
    /// If the playerlist file was provided but could not be parsed, or another
    /// unexpected error occurred, to prevent data loss.
    #[allow(clippy::cognitive_complexity)]
    pub fn load_or_create(args: &Args) -> Self {
        // Playerlist
        let playerlist_path: PathBuf = args
        .playerlist
        .as_ref()
        .map_or_else(Self::locate_playerlist_file, |i| Ok(i.into())).map_err(|e| {
            tracing::error!("Could not find a suitable location for the playerlist: {} \nPlease specify a file path manually with --playerlist otherwise information may not be saved.", e); 
        }).unwrap_or_else(|()| PathBuf::from("playerlist.json"));

        match Self::load_from(playerlist_path) {
            Ok(playerlist) => playerlist,
            Err(ConfigFilesError::Json(path, e)) => {
                tracing::error!("{} could not be loaded: {:?}", path, e);
                tracing::error!(
                    "Please resolve any issues or remove the file, otherwise data may be lost."
                );
                panic!("Failed to load playerlist")
            }
            Err(ConfigFilesError::IO(path, e)) if e.kind() == ErrorKind::NotFound => {
                tracing::warn!("Could not locate {}, creating new playerlist.", &path);
                let mut playerlist = Self::default();
                playerlist.set_path(path.into());
                playerlist
            }
            Err(e) => {
                tracing::error!("Could not load playerlist: {:?}", e);
                tracing::error!(
                    "Please resolve any issues or remove the file, otherwise data may be lost."
                );
                panic!("Failed to load playerlist")
            }
        }
    }

    /// Attempt to load the `PlayerRecords` from the provided file
    ///
    /// # Errors
    /// If the file could not be located, read, or parsed.
    pub fn load_from(path: PathBuf) -> Result<Self, ConfigFilesError> {
        let contents = std::fs::read_to_string(&path)
            .map_err(|e| ConfigFilesError::IO(path.to_string_lossy().into(), e))?;
        let mut playerlist: Self = serde_json::from_str(&contents)
            .map_err(|e| ConfigFilesError::Json(path.to_string_lossy().into(), e))?;
        playerlist.path = path;

        // Map all of the steamids to the records. They were not included when
        // serializing/deserializing the records to prevent duplication in the
        // resulting file.
        for record in &mut playerlist.records.values_mut() {
            // Some old versions had the custom_data set to `null` by default, but an empty
            // object is preferable so I'm using this to fix it lol. It's really
            // not necessary but at the time the UI wasn't a fan of nulls in the
            // custom_data and this fixes it so whatever. :3
            if record.custom_data.is_null() {
                record.custom_data = serde_json::Value::Object(serde_json::Map::new());
            }
        }

        Ok(playerlist)
    }

    /// Attempt to save the `PlayerRecords` to the file it was loaded from
    ///
    /// # Errors
    /// If it failed to serialize or write back to the file.
    pub fn save(&self) -> Result<(), ConfigFilesError> {
        let contents = serde_json::to_string(self).context("Failed to serialize playerlist.")?;
        std::fs::write(&self.path, contents)
            .map_err(|e| ConfigFilesError::IO(self.path.to_string_lossy().into(), e))?;
        Ok(())
    }

    /// Attempt to save the `PlayerRecords`, log errors and ignore result
    pub fn save_ok(&self) {
        if let Err(e) = self.save() {
            tracing::error!("Failed to save playerlist: {:?}", e);
            return;
        }
        // this will never fail to unwrap because the above error would have occured
        // first and broken control flow.
        tracing::debug!("Playerlist saved to {:?}", self.path);
    }

    pub fn set_path(&mut self, path: PathBuf) { self.path = path; }

    /// # Errors
    /// If the config directory could not be located (usually because no valid
    /// home directory was found)
    pub fn locate_playerlist_file() -> Result<PathBuf, ConfigFilesError> {
        Settings::locate_config_directory().map(|dir| dir.join("playerlist.json"))
    }

    pub fn update_name(&mut self, steamid: SteamID, name: Arc<str>) {
        if let Some(record) = self.records.get_mut(&steamid) {
            if !record.previous_names.contains(&name) {
                record.previous_names.push(name);
            }
        }
    }
}

impl Default for PlayerRecords {
    fn default() -> Self {
        let path = Self::locate_playerlist_file()
            .map_err(|e| tracing::warn!("Failed to create config directory: {:?}", e))
            .unwrap_or_else(|()| "playerlist.json".into());

        Self {
            path,
            records: HashMap::new(),
        }
    }
}

impl Deref for PlayerRecords {
    type Target = HashMap<SteamID, PlayerRecord>;

    fn deref(&self) -> &Self::Target { &self.records }
}

impl DerefMut for PlayerRecords {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.records }
}

// PlayerRecord

/// A Record of a player stored in the persistent personal playerlist
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct PlayerRecord {
    pub custom_data: serde_json::Value,
    pub verdict: Verdict,
    pub previous_names: Vec<Arc<str>>,
    /// Time of last manual change made by the user.
    pub modified: DateTime<Utc>,
    pub created: DateTime<Utc>,
}

impl PlayerRecord {
    /// Returns true if the record does not hold any meaningful information
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.verdict == Verdict::Player && {
            self.custom_data.is_null()
                || self.custom_data.as_object().is_some_and(Map::is_empty)
                || self.custom_data.as_array().is_some_and(Vec::is_empty)
                || self.custom_data.as_str().is_some_and(str::is_empty)
        }
    }
}

impl Default for PlayerRecord {
    fn default() -> Self {
        Self {
            custom_data: default_custom_data(),
            verdict: Verdict::default(),
            previous_names: Vec::new(),
            modified: default_date(),
            created: default_date(),
        }
    }
}

#[must_use]
pub fn default_custom_data() -> serde_json::Value { serde_json::Value::Object(Map::new()) }

#[must_use]
pub fn default_date() -> DateTime<Utc> { Utc::now() }

/// What a player is marked as in the personal playerlist
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Player,
    Bot,
    Suspicious,
    Cheater,
    Trusted,
}

impl Display for Verdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{self:?}") }
}

impl Default for Verdict {
    fn default() -> Self { Self::Player }
}
