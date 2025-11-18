use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Pagination {
    #[serde(default = "default_skip")]
    pub skip: u64,
    #[serde(default = "default_limit", deserialize_with = "clamp_limit")]
    pub limit: u64,
}

impl Default for Pagination {
    fn default() -> Self {
        Pagination {
            skip: 0,
            limit: default_limit(),
        }
    }
}

fn default_skip() -> u64 {
    0
}

fn default_limit() -> u64 {
    50
}

fn clamp_limit<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    let limit = u64::deserialize(deserializer)?;

    // Clamp the limit to a maximum of 50 entries.
    Ok(limit.min(default_limit()))
}

impl Into<Pagination> for (u64, u64) {
    fn into(self) -> Pagination {
        Pagination {
            skip: self.0,
            limit: self.1,
        }
    }
}

impl Into<(u64, u64)> for Pagination {
    fn into(self) -> (u64, u64) {
        (self.skip, self.limit)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginationResponse<T> {
    pub items: Vec<T>,
    pub offset: u64,
}
