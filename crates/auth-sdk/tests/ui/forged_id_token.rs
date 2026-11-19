/// Stands in for any deserializer, such as `serde_json::from_str`. A local
/// bound keeps the compiler's note in this file rather than in serde_json,
/// whose source the note quotes only when it was built in the same cache.
fn deserialize<T: for<'de> serde::Deserialize<'de>>() {}

fn main() {
    deserialize::<sourceorigin_auth::IdToken>();
}
