//! Generated, language-neutral CodePet core protocol types.

mod generated;

pub use generated::*;

/// Mechanism-level failure shared by language-neutral transport implementations.
#[derive(Debug)]
pub struct TransportError {
    pub message: String,
}
impl TransportError {
    pub fn new(message: impl ToString) -> Self {
        Self {
            message: message.to_string(),
        }
    }
}
impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for TransportError {}
impl From<TransportError> for ProtocolError {
    fn from(value: TransportError) -> Self {
        // Keep the existing published Provider error code while moving its mechanism.
        Self {
            code: "provider_mux_error".into(),
            message: value.message,
            retryable: false,
            details: None,
        }
    }
}

impl ProtocolVersionOffer {
    /// Validate a continuous compatibility range and choose its highest common version.
    pub fn negotiate(&self, peer: &Self) -> Result<ProtocolVersionSelection, ProtocolError> {
        if self.min_version == 0 || peer.min_version == 0
            || self.min_version > self.version || peer.min_version > peer.version {
            return Err(version_error("invalid_protocol_range", "protocol range must satisfy 1 <= minVersion <= version"));
        }
        let selected_version = self.version.min(peer.version);
        if selected_version < self.min_version.max(peer.min_version) {
            return Err(version_error("unsupported_protocol_version", "protocol ranges do not overlap"));
        }
        Ok(ProtocolVersionSelection { version: self.version, min_version: self.min_version, selected_version })
    }

    /// A responder must select exactly the highest mutually supported version.
    pub fn validate_selection(&self, peer: &ProtocolVersionSelection) -> Result<(), ProtocolError> {
        let selection = self.negotiate(&peer.offer())?;
        if selection.selected_version != peer.selected_version {
            return Err(version_error("invalid_protocol_selection", "peer did not select the highest common version"));
        }
        Ok(())
    }
}
impl ProtocolVersionSelection {
    pub fn offer(&self) -> ProtocolVersionOffer {
        ProtocolVersionOffer { version: self.version, min_version: self.min_version }
    }
}
fn version_error(code: &str, message: &str) -> ProtocolError {
    ProtocolError { code: code.into(), message: message.into(), retryable: false, details: None }
}

/// IDL-driven structural validation shared by generated service codecs.
/// `allow_unknown` applies only to receive enums and discriminated unions.
pub fn validate_protocol_value(schemas: &serde_json::Value, name: &str, value: &serde_json::Value, allow_unknown: bool) -> Result<(), ProtocolError> {
    fn visit(n: &serde_json::Value, v: &serde_json::Value, defs: &serde_json::Value, allow: bool, depth: usize) -> bool {
        if depth > 128 { return false; }
        if let Some(key) = n["ref"].as_str() { return visit(&defs[key],v,defs,allow,depth+1); }
        if let Some(d) = n.get("discriminator") {
            let Some(tag) = v.get(d["field"].as_str().unwrap_or("")).and_then(|v|v.as_str()) else { return false; };
            return match d["variants"].get(tag).and_then(|r|r.as_str()) { Some(key)=>visit(&defs[key],v,defs,allow,depth+1),None=>allow };
        }
        if let Some(variants) = n["oneOf"].as_array() { return variants.iter().filter(|n|visit(n,v,defs,allow,depth+1)).count()==1; }
        if let Some(variants) = n["enum"].as_array() { if !variants.contains(v) && !(allow && variants.len()>1 && v.is_string()) { return false; } }
        match n["type"].as_str() {
            Some("null")=>v.is_null(), Some("boolean")=>v.is_boolean(),
            Some("integer")=> {
                let Some(x)=v.as_i64().map(|x| x as i128).or_else(||v.as_u64().map(|x|x as i128)) else { return false; };
                n["minimum"].as_i64().is_none_or(|m|x>=m as i128) && n["maximum"].as_u64().is_none_or(|m|x<=m as i128)
            },
            Some("string")=> {
                let Some(s)=v.as_str() else { return false; }; let len=s.chars().count() as u64;
                n["minLength"].as_u64().is_none_or(|m|len>=m) && n["maxLength"].as_u64().is_none_or(|m|len<=m)
                    && n["pattern"].as_str().is_none_or(|p| regex::Regex::new(p).is_ok_and(|r|r.is_match(s)))
            },
            Some("array")=> {
                let Some(values)=v.as_array() else { return false; };
                n["minItems"].as_u64().is_none_or(|m|values.len() as u64>=m)
                    && (n["uniqueItems"]!=true || values.iter().enumerate().all(|(i,v)|!values[..i].contains(v)))
                    && values.iter().all(|v|visit(&n["items"],v,defs,allow,depth+1))
            },
            Some("object")=> {
                let Some(values)=v.as_object() else { return false; };
                n["required"].as_array().is_none_or(|required|required.iter().all(|key| key.as_str().is_some_and(|key|values.contains_key(key))))
                    && values.iter().all(|(key,v)| match n["properties"].get(key) {
                        Some(prop)=>visit(prop,v,defs,allow,depth+1), None=> !n["additionalProperties"].is_object() || visit(&n["additionalProperties"],v,defs,allow,depth+1),
                    })
            },
            _=>!n.is_null(),
        }
    }
    if visit(&schemas[name],value,schemas,allow_unknown,0) { Ok(()) }
    else { Err(version_error("protocol_codec_error", &format!("Invalid known fields or unsupported request choice in {name}"))) }
}
