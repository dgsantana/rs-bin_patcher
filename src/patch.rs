use serde::{Deserialize, Serialize};
use hex_buffer_serde::{Hex, HexForm};

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug, Default)]
pub(crate) struct Patch {
    pub sections: Vec<PatchSection>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug, Default)]
pub(crate) struct PatchSection {
    pub id: u32,
    pub start: usize,
    pub end: usize,
    #[serde(with = "HexForm::<Vec<u8>>")]
    pub search: Vec<u8>,
    #[serde(with = "HexForm::<Vec<u8>>")]
    pub data: Vec<u8>,
}
