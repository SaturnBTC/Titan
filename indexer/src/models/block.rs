use {
    bitcoin::{hashes::Hash, BlockHash},
    borsh::{BorshDeserialize, BorshSerialize},
    titan_types_core::TransactionStatus,
};

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct BlockId {
    pub hash: BlockHash,
    pub height: u64,
}

impl BlockId {
    pub fn into_transaction_status(self) -> TransactionStatus {
        TransactionStatus {
            confirmed: true,
            block_height: Some(self.height),
            block_hash: Some(self.hash),
        }
    }
}

pub fn block_id_to_transaction_status(block_id: Option<&BlockId>) -> TransactionStatus {
    match block_id {
        Some(block_id) => block_id.clone().into_transaction_status(),
        None => TransactionStatus::unconfirmed(),
    }
}

impl BorshSerialize for BlockId {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        // First serialize the length of utxos vector
        BorshSerialize::serialize(&(self.hash.as_raw_hash().as_byte_array()), writer)?;
        BorshSerialize::serialize(&self.height, writer)?;

        Ok(())
    }
}

impl BorshDeserialize for BlockId {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        let hash = <[u8; 32]>::deserialize_reader(reader)?;
        let height = u64::deserialize_reader(reader)?;

        Ok(Self {
            hash: bitcoin::BlockHash::from_raw_hash(
                bitcoin::hashes::sha256d::Hash::from_byte_array(hash),
            ),
            height,
        })
    }
}
