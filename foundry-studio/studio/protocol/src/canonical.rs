//! Within `studio-protocol`, `canonical` is the JSON form Agent Studio hashes, and the flow plan's digest taken over
//! it (spec 06 section 3.3), kept in this crate because the API and the runners both link it, so a check's digest and
//! a runner's are taken alike. The form: every object's keys sorted, no whitespace, each number as serde_json writes
//! it, so a value hashes alike however its maps were built (a build that turns on serde_json's `preserve_order` keeps
//! insertion order).

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// Write `json` to `out` in the canonical form.
pub fn write_canonical(json: &Value, out: &mut Vec<u8>) {
    match json {
        Value::Object(fields) => {
            let mut names: Vec<&String> = fields.keys().collect();
            names.sort();
            out.push(b'{');
            for (n, name) in names.into_iter().enumerate() {
                if n > 0 {
                    out.push(b',');
                }
                serde_json::to_writer(&mut *out, name).expect("JSON text always writes into memory");
                out.push(b':');
                write_canonical(&fields[name.as_str()], out);
            }
            out.push(b'}');
        }
        Value::Array(items) => {
            out.push(b'[');
            for (n, item) in items.iter().enumerate() {
                if n > 0 {
                    out.push(b',');
                }
                write_canonical(item, out);
            }
            out.push(b']');
        }
        scalar => serde_json::to_writer(&mut *out, scalar).expect("JSON text always writes into memory"),
    }
}

/// A flow plan's digest: `sha256:` and the SHA-256, in 64 hex digits, of the canonical JSON of what the run executes,
/// the plan's composition and its settings block as the runner writes them into its `spec.json`, and nothing else,
/// so neither a label, the read's provenance nor the compiler version moves it.
pub fn flow_plan_digest(composition: &Value, settings: &Value) -> String {
    let mut canonical = Vec::new();
    write_canonical(&json!({ "composition": composition, "settings": settings }), &mut canonical);
    format!("sha256:{:x}", Sha256::digest(&canonical))
}
