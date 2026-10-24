//! Encoding of plugin state stored in projects.
//!
//! The project keeps a plugin's state as one opaque string:
//! `vst3:<component hex>` or `vst3:<component hex>:<controller hex>`.

pub const PREFIX: &str = "vst3:";

pub fn encode(component: &str, controller: Option<&str>) -> String {
    match controller {
        Some(c) if !c.is_empty() => format!("{PREFIX}{component}:{c}"),
        _ => format!("{PREFIX}{component}"),
    }
}

/// Returns (component hex, controller hex). `None` for unknown formats.
pub fn decode(s: &str) -> Option<(String, Option<String>)> {
    let rest = s.strip_prefix(PREFIX)?;
    let mut it = rest.splitn(2, ':');
    let comp = it.next()?.to_string();
    if !comp.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let ctl = it.next().filter(|c| !c.is_empty() && c.bytes().all(|b| b.is_ascii_hexdigit())).map(String::from);
    Some((comp, ctl))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        assert_eq!(decode(&encode("00ff", None)), Some(("00ff".into(), None)));
        assert_eq!(decode(&encode("00ff", Some("ab"))), Some(("00ff".into(), Some("ab".into()))));
        assert_eq!(decode("garbage"), None);
        assert_eq!(decode("vst3:zz"), None);
    }
}
