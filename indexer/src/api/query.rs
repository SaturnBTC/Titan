use {
    crate::index::{Index, IndexError},
    bitcoin::BlockHash,
    std::sync::Arc,
    titan_types_api::query,
    titan_types_core::RuneId,
};

pub fn to_hash(block: &query::Block, index: &Arc<Index>) -> Result<BlockHash, IndexError> {
    match block {
        query::Block::Height(height) => index.get_block_hash(*height),
        query::Block::Hash(hash) => Ok(*hash),
    }
}

pub fn to_rune_id(rune: &query::Rune, index: &Arc<Index>) -> Result<RuneId, IndexError> {
    match rune {
        query::Rune::Spaced(spaced_rune) => index.get_rune_id(&spaced_rune.rune),
        query::Rune::Id(rune_id) => Ok(*rune_id),
    }
}
