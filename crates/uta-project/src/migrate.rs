//! Project document migrations.
//!
//! Documents are migrated as untyped JSON, one version step at a time, before
//! being deserialized into the current [`crate::model::Project`]. Every step is
//! a pure function so it can be tested in isolation. Never edit an existing
//! step after release — add a new one.
//!
//! History:
//! * v1 — internal pre-release format: track level stored as a linear
//!   `volume` factor; no take groups or stereo-input flag.
//! * v2 — track level in dB (`gainDb`), `takeGroups`, `inputStereo`.

use serde_json::{json, Value};

use crate::error::StoreError;
use crate::model::FORMAT_VERSION;

/// Migrate a document to [`FORMAT_VERSION`]. Returns the migrated document
/// and the version it started from.
pub fn migrate(mut doc: Value) -> Result<(Value, u32), StoreError> {
    let found = doc
        .get("formatVersion")
        .and_then(Value::as_u64)
        .ok_or_else(|| StoreError::Other("project document has no formatVersion".into()))? as u32;
    if found > FORMAT_VERSION {
        return Err(StoreError::TooNew {
            found,
            supported: FORMAT_VERSION,
        });
    }
    if found == 0 {
        return Err(StoreError::Other("invalid formatVersion 0".into()));
    }
    let mut v = found;
    while v < FORMAT_VERSION {
        doc = match v {
            1 => v1_to_v2(doc)?,
            other => return Err(StoreError::Other(format!("no migration from format {other}"))),
        };
        v += 1;
        doc["formatVersion"] = json!(v);
    }
    Ok((doc, found))
}

fn v1_to_v2(mut doc: Value) -> Result<Value, StoreError> {
    if let Some(tracks) = doc.get_mut("tracks").and_then(Value::as_array_mut) {
        for t in tracks {
            let obj = t
                .as_object_mut()
                .ok_or_else(|| StoreError::Other("track is not an object".into()))?;
            let volume = obj.remove("volume").and_then(|v| v.as_f64()).unwrap_or(1.0);
            let db = if volume <= 0.0 { -96.0 } else { (20.0 * volume.log10()).max(-96.0) };
            obj.insert("gainDb".into(), json!(db));
            obj.entry("takeGroups").or_insert(json!([]));
            obj.entry("inputStereo").or_insert(json!(false));
        }
    }
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Project;

    #[test]
    fn v1_document_migrates_and_loads() {
        let mut current = serde_json::to_value(Project::new("Old", 44_100)).unwrap();
        // build a v1 document from a current one with one track
        let mut p = Project::new("Old", 44_100);
        crate::edit::add_track(&mut p, Some("Lead".into()));
        current = serde_json::to_value(&p).unwrap_or(current);
        let t = current["tracks"][0].as_object_mut().unwrap();
        t.remove("gainDb");
        t.remove("takeGroups");
        t.remove("inputStereo");
        t.insert("volume".into(), json!(0.5));
        current["formatVersion"] = json!(1);

        let (doc, from) = migrate(current).unwrap();
        assert_eq!(from, 1);
        let p: Project = serde_json::from_value(doc).unwrap();
        assert_eq!(p.format_version, FORMAT_VERSION);
        assert!((p.tracks[0].gain_db + 6.0206).abs() < 1e-3);
        assert!(p.tracks[0].take_groups.is_empty());
    }

    #[test]
    fn rejects_newer_and_invalid() {
        let mut doc = serde_json::to_value(Project::new("x", 48_000)).unwrap();
        doc["formatVersion"] = json!(FORMAT_VERSION + 1);
        assert!(matches!(migrate(doc), Err(StoreError::TooNew { .. })));
        assert!(migrate(json!({"name": "no version"})).is_err());
    }
}
