//! Effect presets stored in the library database.

use std::collections::BTreeMap;

use uta_project::library::PresetEntry;

use crate::error::{CoreError, Result};
use crate::{now, AppCore};

impl AppCore {
    pub fn presets(&self, effect_kind: Option<&str>) -> Result<Vec<PresetEntry>> {
        Ok(self.library.lock().presets(effect_kind)?)
    }

    /// Save (or overwrite by name) a preset for a built-in effect. Unknown
    /// parameter ids are rejected so presets stay loadable.
    pub fn save_preset(&self, effect_kind: &str, name: &str, params: BTreeMap<String, f32>) -> Result<PresetEntry> {
        let kind = uta_dsp::BuiltinKind::from_id(effect_kind).ok_or_else(|| CoreError::Invalid(format!("unknown effect {effect_kind}")))?;
        let name = name.trim();
        if name.is_empty() {
            return Err(CoreError::Invalid("preset name cannot be empty".into()));
        }
        let mut clean = BTreeMap::new();
        for (k, v) in params {
            let info = kind.params().iter().find(|p| p.id == k).ok_or_else(|| CoreError::Invalid(format!("unknown parameter {k}")))?;
            clean.insert(k, info.clamp(v));
        }
        let p = PresetEntry {
            id: uuid::Uuid::new_v4().to_string(),
            effect_kind: kind.id().into(),
            name: name.chars().take(60).collect(),
            params: serde_json::to_value(clean)?,
            created_at: now(),
        };
        self.library.lock().save_preset(&p)?;
        Ok(p)
    }

    pub fn delete_preset(&self, id: &str) -> Result<()> {
        Ok(self.library.lock().delete_preset(id)?)
    }

    /// Built-in effect descriptions for the UI (ids, ranges, defaults).
    pub fn effect_catalog(&self) -> serde_json::Value {
        serde_json::Value::Array(
            uta_dsp::BuiltinKind::ALL
                .iter()
                .map(|k| serde_json::json!({ "id": k.id(), "name": k.display_name(), "params": k.params() }))
                .collect(),
        )
    }
}
