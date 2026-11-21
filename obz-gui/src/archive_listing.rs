use obz_core::engine::Entry;
use serde::ser::{SerializeSeq, Serializer};
use serde::Serialize;

pub(crate) struct Listing<'a>(pub &'a [Entry]);

impl Serialize for Listing<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut rows = serializer.serialize_seq(Some(self.0.len()))?;
        for entry in self.0 {
            rows.serialize_element(&Row(entry))?;
        }
        rows.end()
    }
}

struct Row<'a>(&'a Entry);

impl Serialize for Row<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let entry = self.0;
        let flags = u8::from(entry.is_dir)
            | (u8::from(entry.encrypted) << 1)
            | (u8::from(entry.size_known) << 2);
        // Preserve every field, omitting only trailing empty strings.
        let metadata = [
            &entry.crc, &entry.modified, &entry.attributes, &entry.method,
            &entry.created, &entry.accessed, &entry.comment,
        ];
        let count = metadata.iter().rposition(|value| !value.is_empty()).map_or(0, |i| i + 1);
        let mut row = serializer.serialize_seq(Some(4 + count))?;
        row.serialize_element(&entry.name)?;
        row.serialize_element(&flags)?;
        row.serialize_element(&entry.size)?;
        row.serialize_element(&entry.packed_size)?;
        for value in &metadata[..count] {
            row.serialize_element(value)?;
        }
        row.end()
    }
}

pub(crate) fn response(payload: impl Serialize) -> Result<tauri::ipc::Response, String> {
    serde_json::to_vec(&payload)
        .map(tauri::ipc::Response::new)
        .map_err(|error| error.to_string())
}
