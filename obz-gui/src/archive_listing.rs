use obz_core::engine::Entry;
use serde::ser::{SerializeSeq, Serializer};
use serde::Serialize;

pub(crate) struct Listing<'a>(pub &'a [Entry]);

const REUSE_PARENT: u8 = 1 << 3;

impl Serialize for Listing<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut rows = serializer.serialize_seq(Some(self.0.len()))?;
        let mut previous_parent = "";
        for entry in self.0 {
            let parent_end = entry.name.rfind(['/', '\\']).map_or(0, |end| end + 1);
            let parent = &entry.name[..parent_end];
            let reused_prefix = if !parent.is_empty() && parent == previous_parent {
                parent_end
            } else {
                0
            };
            rows.serialize_element(&Row(entry, reused_prefix))?;
            previous_parent = parent;
        }
        rows.end()
    }
}

struct Row<'a>(&'a Entry, usize);

impl Serialize for Row<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let entry = self.0;
        let flags = u8::from(entry.is_dir)
            | (u8::from(entry.encrypted) << 1)
            | (u8::from(entry.size_known) << 2)
            | if self.1 > 0 { REUSE_PARENT } else { 0 };
        // Preserve every field, omitting only trailing empty strings.
        let metadata = [
            &entry.crc, &entry.modified, &entry.attributes, &entry.method,
            &entry.created, &entry.accessed, &entry.comment,
        ];
        let count = metadata.iter().rposition(|value| !value.is_empty()).map_or(0, |i| i + 1);
        let mut row = serializer.serialize_seq(Some(4 + count))?;
        row.serialize_element(&entry.name[self.1..])?;
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
