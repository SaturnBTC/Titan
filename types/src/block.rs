use {
    crate::{rune_id::RuneId, SerializedTxid},
    bitcoin::{
        block::{Header, Version},
        hashes::Hash,
        BlockHash, CompactTarget, TxMerkleNode,
    },
    borsh::{BorshDeserialize, BorshSerialize},
    serde::{Deserialize, Serialize},
    std::io::{Read, Result, Write},
};

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct Block {
    pub height: u64,
    pub header: Header,
    pub tx_ids: Vec<SerializedTxid>,
    pub etched_runes: Vec<RuneId>,
}

impl BorshSerialize for Block {
    fn serialize<W: Write>(&self, writer: &mut W) -> Result<()> {
        // 1) Serialize `height`
        BorshSerialize::serialize(&self.height, writer)?;

        // 2) Serialize `header` manually
        BorshSerialize::serialize(&self.header.version.to_consensus(), writer)?;

        // `as_raw_hash()` returns a &sha256d::Hash, which we can then convert
        // into a 32-byte array. That array is what Borsh will see.
        BorshSerialize::serialize(
            &self.header.prev_blockhash.as_raw_hash().as_byte_array(),
            writer,
        )?;

        BorshSerialize::serialize(
            &self.header.merkle_root.as_raw_hash().as_byte_array(),
            writer,
        )?;
        BorshSerialize::serialize(&self.header.time, writer)?;
        BorshSerialize::serialize(&self.header.bits.to_consensus(), writer)?;
        BorshSerialize::serialize(&self.header.nonce, writer)?;

        // 3) Serialize `tx_ids` as a Vec<SerializedTxid>
        //    (Vec<SerializedTxid> already implements BorshSerialize)
        BorshSerialize::serialize(&self.tx_ids, writer)?;

        // 3) Serialize `etched_runes` manually:
        //    Borsh doesn't know about `RuneId`, so we store it ourselves:
        //
        //    - First, write the length of the vector
        //    - Then for each `RuneId`, write out (block, tx)

        let etched_len = self.etched_runes.len() as u64;
        BorshSerialize::serialize(&etched_len, writer)?;

        for rune_id in &self.etched_runes {
            BorshSerialize::serialize(&rune_id.block, writer)?;
            BorshSerialize::serialize(&rune_id.tx, writer)?;
        }

        Ok(())
    }
}

impl BorshDeserialize for Block {
    fn deserialize_reader<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        // 1) Deserialize `height`
        let height = u64::deserialize_reader(reader)?;

        // 2) Deserialize `header` manually
        let version = Version::from_consensus(i32::deserialize_reader(reader)?);

        let mut prev_hash = [0u8; 32];
        reader.read_exact(&mut prev_hash)?;
        let prev_blockhash =
            BlockHash::from_raw_hash(Hash::from_slice(&prev_hash).map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid prev_blockhash")
            })?);

        let mut merkle = [0u8; 32];
        reader.read_exact(&mut merkle)?;
        let merkle_root = TxMerkleNode::from_raw_hash(Hash::from_slice(&merkle).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid merkle_root")
        })?);

        let time = u32::deserialize_reader(reader)?;
        let bits = CompactTarget::from_consensus(u32::deserialize_reader(reader)?);
        let nonce = u32::deserialize_reader(reader)?;

        let header = Header {
            version,
            prev_blockhash,
            merkle_root,
            time,
            bits,
            nonce,
        };

        // 3) Deserialize `tx_ids` (Vec<String>)
        let tx_ids = Vec::<SerializedTxid>::deserialize_reader(reader)?;

        // 3) Deserialize `etched_runes` manually:
        //    - Read the length
        //    - For each entry, read `block` (u64) then `tx` (u32)

        let etched_len = u64::deserialize_reader(reader)?;
        let mut etched_runes = Vec::with_capacity(etched_len as usize);

        for _ in 0..etched_len {
            let block = u64::deserialize_reader(reader)?;
            let tx = u32::deserialize_reader(reader)?;
            etched_runes.push(RuneId::new(block, tx));
        }

        Ok(Self {
            height,
            header,
            tx_ids,
            etched_runes,
        })
    }
}

impl Block {
    pub fn empty_block(height: u64, header: Header) -> Self {
        Self {
            height,
            header,
            tx_ids: vec![],
            etched_runes: vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rune_id::RuneId;
    use bitcoin::{block::Version, hashes::Hash, BlockHash, CompactTarget, TxMerkleNode};
    use borsh::BorshDeserialize;

    /// Helper function to create a sample Header for testing
    fn create_test_header() -> Header {
        Header {
            version: Version::from_consensus(1),
            prev_blockhash: BlockHash::from_raw_hash(Hash::from_slice(&[1u8; 32]).unwrap()),
            merkle_root: TxMerkleNode::from_raw_hash(Hash::from_slice(&[2u8; 32]).unwrap()),
            time: 1640995200, // 2022-01-01 00:00:00 UTC
            bits: CompactTarget::from_consensus(0x1d00ffff),
            nonce: 42,
        }
    }

    /// Helper function to create test SerializedTxids
    fn create_test_txids() -> Vec<SerializedTxid> {
        vec![
            SerializedTxid::from([3u8; 32]),
            SerializedTxid::from([4u8; 32]),
            SerializedTxid::from([5u8; 32]),
        ]
    }

    /// Helper function to create test RuneIds
    fn create_test_rune_ids() -> Vec<RuneId> {
        vec![
            RuneId::new(100, 0),
            RuneId::new(101, 1),
            RuneId::new(102, 2),
        ]
    }

    #[test]
    fn test_empty_block_serialization_roundtrip() {
        let header = create_test_header();
        let block = Block::empty_block(12345, header.clone());

        // Serialize
        let serialized = borsh::to_vec(&block).expect("Failed to serialize empty block");

        // Deserialize
        let deserialized =
            Block::try_from_slice(&serialized).expect("Failed to deserialize empty block");

        // Verify all fields match
        assert_eq!(block.height, deserialized.height);
        assert_eq!(block.header, deserialized.header);
        assert_eq!(block.tx_ids, deserialized.tx_ids);
        assert_eq!(block.etched_runes, deserialized.etched_runes);
        assert_eq!(block, deserialized);
    }

    #[test]
    fn test_full_block_serialization_roundtrip() {
        let header = create_test_header();
        let tx_ids = create_test_txids();
        let etched_runes = create_test_rune_ids();

        let block = Block {
            height: 67890,
            header: header.clone(),
            tx_ids: tx_ids.clone(),
            etched_runes: etched_runes.clone(),
        };

        // Serialize
        let serialized = borsh::to_vec(&block).expect("Failed to serialize full block");

        // Deserialize
        let deserialized =
            Block::try_from_slice(&serialized).expect("Failed to deserialize full block");

        // Verify all fields match
        assert_eq!(block.height, deserialized.height);
        assert_eq!(block.header, deserialized.header);
        assert_eq!(block.tx_ids, deserialized.tx_ids);
        assert_eq!(block.etched_runes, deserialized.etched_runes);
        assert_eq!(block, deserialized);
    }

    #[test]
    fn test_block_with_single_tx_serialization() {
        let header = create_test_header();
        let tx_ids = vec![SerializedTxid::from([6u8; 32])];

        let block = Block {
            height: 1,
            header: header.clone(),
            tx_ids: tx_ids.clone(),
            etched_runes: vec![],
        };

        // Serialize
        let serialized = borsh::to_vec(&block).expect("Failed to serialize single tx block");

        // Deserialize
        let deserialized =
            Block::try_from_slice(&serialized).expect("Failed to deserialize single tx block");

        assert_eq!(block, deserialized);
        assert_eq!(deserialized.tx_ids.len(), 1);
        assert_eq!(deserialized.etched_runes.len(), 0);
    }

    #[test]
    fn test_block_with_single_rune_serialization() {
        let header = create_test_header();
        let etched_runes = vec![RuneId::new(999, 888)];

        let block = Block {
            height: 2,
            header: header.clone(),
            tx_ids: vec![],
            etched_runes: etched_runes.clone(),
        };

        // Serialize
        let serialized = borsh::to_vec(&block).expect("Failed to serialize single rune block");

        // Deserialize
        let deserialized =
            Block::try_from_slice(&serialized).expect("Failed to deserialize single rune block");

        assert_eq!(block, deserialized);
        assert_eq!(deserialized.tx_ids.len(), 0);
        assert_eq!(deserialized.etched_runes.len(), 1);
        assert_eq!(deserialized.etched_runes[0].block, 999);
        assert_eq!(deserialized.etched_runes[0].tx, 888);
    }

    #[test]
    fn test_block_with_edge_case_values() {
        let header = Header {
            version: Version::from_consensus(i32::MAX),
            prev_blockhash: BlockHash::from_raw_hash(Hash::from_slice(&[0u8; 32]).unwrap()),
            merkle_root: TxMerkleNode::from_raw_hash(Hash::from_slice(&[255u8; 32]).unwrap()),
            time: u32::MAX,
            bits: CompactTarget::from_consensus(u32::MAX),
            nonce: u32::MAX,
        };

        let block = Block {
            height: u64::MAX,
            header: header.clone(),
            tx_ids: vec![SerializedTxid::all_zeros()],
            etched_runes: vec![RuneId::new(u64::MAX, u32::MAX)],
        };

        // Serialize
        let serialized = borsh::to_vec(&block).expect("Failed to serialize edge case block");

        // Deserialize
        let deserialized =
            Block::try_from_slice(&serialized).expect("Failed to deserialize edge case block");

        assert_eq!(block, deserialized);
        assert_eq!(deserialized.height, u64::MAX);
        assert_eq!(deserialized.header.time, u32::MAX);
        assert_eq!(deserialized.etched_runes[0].block, u64::MAX);
        assert_eq!(deserialized.etched_runes[0].tx, u32::MAX);
    }

    #[test]
    fn test_large_vectors_serialization() {
        let header = create_test_header();

        // Create large vectors to test performance and correctness
        let large_tx_ids: Vec<SerializedTxid> = (0..1000)
            .map(|i| {
                let mut bytes = [0u8; 32];
                bytes[0..4].copy_from_slice(&(i as u32).to_le_bytes());
                SerializedTxid::from(bytes)
            })
            .collect();

        let large_etched_runes: Vec<RuneId> = (0..500)
            .map(|i| RuneId::new(i as u64, (i * 2) as u32))
            .collect();

        let block = Block {
            height: 500000,
            header: header.clone(),
            tx_ids: large_tx_ids.clone(),
            etched_runes: large_etched_runes.clone(),
        };

        // Serialize
        let serialized = borsh::to_vec(&block).expect("Failed to serialize large block");

        // Deserialize
        let deserialized =
            Block::try_from_slice(&serialized).expect("Failed to deserialize large block");

        assert_eq!(block, deserialized);
        assert_eq!(deserialized.tx_ids.len(), 1000);
        assert_eq!(deserialized.etched_runes.len(), 500);

        // Verify first and last elements
        assert_eq!(deserialized.tx_ids[0], large_tx_ids[0]);
        assert_eq!(deserialized.tx_ids[999], large_tx_ids[999]);
        assert_eq!(deserialized.etched_runes[0], large_etched_runes[0]);
        assert_eq!(deserialized.etched_runes[499], large_etched_runes[499]);
    }

    #[test]
    fn test_serialized_data_integrity() {
        let header = create_test_header();
        let block = Block {
            height: 123456,
            header: header.clone(),
            tx_ids: create_test_txids(),
            etched_runes: create_test_rune_ids(),
        };

        // Serialize twice and ensure identical results
        let serialized1 = borsh::to_vec(&block).expect("Failed first serialization");
        let serialized2 = borsh::to_vec(&block).expect("Failed second serialization");

        assert_eq!(
            serialized1, serialized2,
            "Serialization should be deterministic"
        );

        // Verify serialized data is not empty
        assert!(
            !serialized1.is_empty(),
            "Serialized data should not be empty"
        );

        // Deserialize and re-serialize to test full roundtrip
        let deserialized = Block::try_from_slice(&serialized1).expect("Failed to deserialize");
        let re_serialized = borsh::to_vec(&deserialized).expect("Failed to re-serialize");

        assert_eq!(
            serialized1, re_serialized,
            "Re-serialization should match original"
        );
    }

    #[test]
    fn test_header_fields_preservation() {
        // Test various header configurations to ensure all fields are preserved
        let headers = vec![
            Header {
                version: Version::from_consensus(1),
                prev_blockhash: BlockHash::from_raw_hash(Hash::from_slice(&[1u8; 32]).unwrap()),
                merkle_root: TxMerkleNode::from_raw_hash(Hash::from_slice(&[2u8; 32]).unwrap()),
                time: 1000000,
                bits: CompactTarget::from_consensus(0x1d00ffff),
                nonce: 12345,
            },
            Header {
                version: Version::from_consensus(536870912), // Version 2
                prev_blockhash: BlockHash::from_raw_hash(Hash::from_slice(&[255u8; 32]).unwrap()),
                merkle_root: TxMerkleNode::from_raw_hash(Hash::from_slice(&[0u8; 32]).unwrap()),
                time: 0,
                bits: CompactTarget::from_consensus(0x1d00ffff),
                nonce: 0,
            },
        ];

        for (i, header) in headers.iter().enumerate() {
            let block = Block {
                height: i as u64,
                header: *header,
                tx_ids: vec![],
                etched_runes: vec![],
            };

            let serialized =
                borsh::to_vec(&block).expect(&format!("Failed to serialize header test {}", i));
            let deserialized = Block::try_from_slice(&serialized)
                .expect(&format!("Failed to deserialize header test {}", i));

            assert_eq!(block.header.version, deserialized.header.version);
            assert_eq!(
                block.header.prev_blockhash,
                deserialized.header.prev_blockhash
            );
            assert_eq!(block.header.merkle_root, deserialized.header.merkle_root);
            assert_eq!(block.header.time, deserialized.header.time);
            assert_eq!(block.header.bits, deserialized.header.bits);
            assert_eq!(block.header.nonce, deserialized.header.nonce);
        }
    }

    #[test]
    fn test_invalid_data_handling() {
        // Test with incomplete data
        let incomplete_data = vec![1, 2, 3, 4, 5];
        assert!(Block::try_from_slice(&incomplete_data).is_err());

        // Test with empty data
        let empty_data = vec![];
        assert!(Block::try_from_slice(&empty_data).is_err());
    }
}
