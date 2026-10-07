use crate::{platform::HotkeySpec, timing::TimingConfig};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActivationMode {
    #[default]
    Cursor,
    Hotkey,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum NewlineMode {
    #[default]
    Enter,
    ShiftEnter,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub version: u32,
    pub text: String,
    pub timing: TimingConfig,
    pub mode: ActivationMode,
    pub newline: NewlineMode,
    pub startup_seconds: f64,
    pub hotkey: HotkeySpec,
    pub target_minutes: String,
    pub pinned: bool,
    pub collapsed: bool,
    pub text_split: f32,
    pub glass_tint: u8,
    pub blur: bool,
    pub glass_revision: u32,
    pub position: Option<[f32; 2]>,
    pub expanded_size: [f32; 2],
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            version: 1,
            text: String::new(),
            timing: TimingConfig::default(),
            mode: ActivationMode::Cursor,
            newline: NewlineMode::Enter,
            startup_seconds: 5.,
            hotkey: HotkeySpec::default(),
            target_minutes: String::new(),
            pinned: false,
            collapsed: false,
            text_split: 0.28,
            glass_tint: 140,
            blur: true,
            glass_revision: 4,
            position: None,
            expanded_size: [460., 740.],
        }
    }
}

impl AppSettings {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 {
            return Err("This settings file uses an unsupported version.".into());
        }
        self.timing.validate()?;
        self.hotkey.validate()?;
        if !self.text_split.is_finite() || !(0.12..=0.78).contains(&self.text_split) {
            return Err("The text/timing divider position is invalid.".into());
        }
        if !self.startup_seconds.is_finite() || !(0. ..=60.).contains(&self.startup_seconds) {
            return Err("Startup delay must be between 0 and 60 seconds.".into());
        }
        if !self
            .expanded_size
            .iter()
            .all(|v| v.is_finite() && (360. ..=2000.).contains(v))
            || self
                .position
                .is_some_and(|p| !p.iter().all(|v| v.is_finite()))
        {
            return Err("Window geometry is invalid.".into());
        }
        Ok(())
    }
}

pub fn settings_path() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("ShardTyper")
        .join("settings.json")
}

pub fn load(path: &Path) -> (AppSettings, Option<String>) {
    match fs::read(path) {
        Ok(bytes) => match serde_json::from_slice::<serde_json::Value>(&bytes)
            .map_err(|e| e.to_string())
            .and_then(|value| {
                let old_tab_layout =
                    value.get("tab").is_some() && value.get("text_split").is_none();
                let old_glass = value
                    .get("glass_revision")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0)
                    < 4;
                let mut settings: AppSettings =
                    serde_json::from_value(value).map_err(|e| e.to_string())?;
                if old_tab_layout {
                    // Give the combined view room while keeping the user's
                    // text, timing, position, and other preferences intact.
                    settings.expanded_size[1] = settings.expanded_size[1].max(740.);
                }
                if old_glass {
                    if (settings.text_split - 0.30).abs() < 0.001 {
                        settings.text_split = 0.28;
                    }
                    settings.glass_tint = 140;
                    settings.blur = true;
                    settings.glass_revision = 4;
                }
                settings.validate().map(|_| settings)
            }) {
            Ok(s) => (s, None),
            Err(e) => (
                AppSettings::default(),
                Some(format!(
                    "Settings could not be restored: {e} Your original file will be preserved as settings.broken.json."
                )),
            ),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (AppSettings::default(), None),
        Err(e) => (
            AppSettings::default(),
            Some(format!("Could not read local settings: {e}")),
        ),
    }
}

pub fn save(path: &Path, settings: &AppSettings) -> Result<(), String> {
    settings.validate()?;
    let parent = path
        .parent()
        .ok_or("Settings path has no parent directory.")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let tmp = parent.join(format!("settings.{}.tmp", std::process::id()));
    let bytes = serde_json::to_vec_pretty(settings).map_err(|e| e.to_string())?;
    let mut file = fs::File::create(&tmp).map_err(|e| e.to_string())?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(file);
    fs::rename(&tmp, path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn atomic_save_roundtrip_and_corrupt_recovery() {
        let path = std::env::temp_dir()
            .join(format!("shard-settings-test-{}", std::process::id()))
            .join("settings.json");
        let s = AppSettings {
            text: "icy 👩‍💻\nwords".into(),
            collapsed: true,
            ..Default::default()
        };
        save(&path, &s).unwrap();
        save(&path, &s).unwrap();
        let (loaded, notice) = load(&path);
        assert_eq!(loaded, s);
        assert!(notice.is_none());
        fs::write(&path, b"broken").unwrap();
        assert!(load(&path).1.is_some());
        fs::remove_file(&path).unwrap();
        fs::remove_dir(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn old_tab_settings_migrate_without_losing_text_or_timing() {
        let path =
            std::env::temp_dir().join(format!("shard-migration-{}.json", std::process::id()));
        let mut value = serde_json::to_value(AppSettings {
            text: "keep my words".into(),
            expanded_size: [440., 600.],
            ..Default::default()
        })
        .unwrap();
        value.as_object_mut().unwrap().remove("text_split");
        value.as_object_mut().unwrap().remove("glass_revision");
        value["glass_tint"] = serde_json::json!(38);
        value["blur"] = serde_json::json!(false);
        value["tab"] = serde_json::json!("Timing");
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let (loaded, notice) = load(&path);
        assert!(notice.is_none());
        assert_eq!(loaded.text, "keep my words");
        assert_eq!(loaded.timing, TimingConfig::default());
        assert_eq!(loaded.expanded_size, [440., 740.]);
        assert_eq!(loaded.text_split, AppSettings::default().text_split);
        assert_eq!(loaded.glass_tint, 140);
        assert!(loaded.blur);
        assert_eq!(loaded.glass_revision, 4);
        fs::remove_file(&path).unwrap();
    }
}
