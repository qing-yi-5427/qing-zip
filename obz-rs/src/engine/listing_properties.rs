//! Common technical properties have fixed slots; unknown properties still
//! participate in duplicate detection, including properties not shown by the GUI.
use std::collections::HashSet;

#[derive(Clone, Copy)]
pub(super) enum Field {
    Path,
    Size,
    PackedSize,
    Folder,
    Attributes,
    Crc,
    Encrypted,
    Modified,
    Method,
    Comment,
    SymbolicLink,
    HardLink,
    AlternateStream,
    Created,
    Accessed,
    Characteristics,
    HostOs,
    Version,
    VolumeIndex,
    Offset,
    Count,
}

impl Field {
    fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "Path" => Self::Path,
            "Size" => Self::Size,
            "Packed Size" => Self::PackedSize,
            "Folder" => Self::Folder,
            "Attributes" => Self::Attributes,
            "CRC" => Self::Crc,
            "Encrypted" => Self::Encrypted,
            "Modified" => Self::Modified,
            "Method" => Self::Method,
            "Comment" => Self::Comment,
            "Symbolic Link" => Self::SymbolicLink,
            "Hard Link" => Self::HardLink,
            "Alternate Stream" => Self::AlternateStream,
            "Created" => Self::Created,
            "Accessed" => Self::Accessed,
            "Characteristics" => Self::Characteristics,
            "Host OS" => Self::HostOs,
            "Version" => Self::Version,
            "Volume Index" => Self::VolumeIndex,
            "Offset" => Self::Offset,
            _ => return None,
        })
    }
}

#[derive(Default)]
pub(super) struct Properties<'a> {
    values: [Option<&'a str>; Field::Count as usize],
    other: HashSet<&'a str>,
}

impl<'a> Properties<'a> {
    pub(super) fn insert(&mut self, name: &'a str, value: &'a str) -> bool {
        if let Some(field) = Field::parse(name) {
            self.values[field as usize].replace(value).is_none()
        } else {
            self.other.insert(name)
        }
    }

    pub(super) fn get(&self, field: Field) -> Option<&'a str> {
        self.values[field as usize]
    }

    pub(super) fn clear(&mut self) {
        self.values.fill(None);
        if self.other.capacity() > 128 {
            self.other = HashSet::new();
        } else {
            self.other.clear();
        }
    }
}
