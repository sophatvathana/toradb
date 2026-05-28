use serde::{Deserialize, Serialize};
use toradb_core::CandidateSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum Request {
    Health,
    SegmentSearch {
        table: String,
        segment: u32,
        query: String,
        k: u32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Response {
    Ok {
        #[serde(default)]
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        candidates: Option<CandidateSet>,
    },
    Error {
        message: String,
    },
}

impl Response {
    pub fn ok_candidates(candidates: CandidateSet) -> Self {
        Self::Ok {
            message: String::new(),
            candidates: Some(candidates),
        }
    }

    pub fn ok_message(message: impl Into<String>) -> Self {
        Self::Ok {
            message: message.into(),
            candidates: None,
        }
    }

    pub fn err(message: impl Into<String>) -> Self {
        Self::Error {
            message: message.into(),
        }
    }
}
