use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug, Default)]
pub(crate) struct Patch {
    pub sections: Vec<PatchSection>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug, Default)]
pub(crate) struct PatchSection {
    pub id: u32,
    pub start: usize,
    pub end: usize,
    #[serde(with = "serde_bytes")]
    pub search: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub data: Vec<u8>,
}
