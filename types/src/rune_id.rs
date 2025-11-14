use std::{fmt::Display, str::FromStr};

use borsh::{BorshDeserialize, BorshSerialize};
use bytemuck::{Pod, Zeroable};
use serde::{Deserialize, Serialize};

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, Hash, Ord, PartialOrd, Pod, Zeroable)]
#[repr(C)]
pub struct RuneId {
    pub block: u64,
    pub tx: u32,
    _padding: [u8; 4],
}

impl RuneId {
    pub const BTC: Self = RuneId {
        block: 0,
        tx: 0,
        _padding: [0; 4],
    };

    pub fn new(block: u64, tx: u32) -> Self {
        RuneId {
            block,
            tx,
            _padding: [0; 4],
        }
    }

    /// Returns token bytes as a fixed-size array without heap allocation
    pub fn to_bytes(&self) -> [u8; 12] {
        let mut result = [0u8; 12];
        result[0..8].copy_from_slice(&self.block.to_le_bytes());
        result[8..12].copy_from_slice(&self.tx.to_le_bytes());
        result
    }

    /// Deterministically sort the two `RuneId`s and return their little-endian
    /// byte representations.
    ///
    /// This is the canonical way we ensure that the *same* pair of rune always
    /// maps to the *same* PDA seeds, regardless of call-site ordering.
    /// The function is `const`-friendly and completely stack-allocated so it can be
    /// evaluated at compile-time in tests.
    pub fn get_sorted_rune_ids(rune0: &RuneId, rune1: &RuneId) -> ([u8; 12], [u8; 12]) {
        let rune0_bytes = rune0.to_bytes();
        let rune1_bytes = rune1.to_bytes();
        if rune0_bytes <= rune1_bytes {
            (rune0_bytes, rune1_bytes)
        } else {
            (rune1_bytes, rune0_bytes)
        }
    }

    /// Calculate the delta between two RuneIds (same as ordinals implementation)
    pub fn delta(self, next: RuneId) -> Option<(u128, u128)> {
        let block = next.block.checked_sub(self.block)?;

        let tx = if block == 0 {
            next.tx.checked_sub(self.tx)?
        } else {
            next.tx
        };

        Some((block.into(), tx.into()))
    }

    /// Calculate the next RuneId given deltas (same as ordinals implementation)
    pub fn next(self, block: u128, tx: u128) -> Option<RuneId> {
        Some(RuneId::new(
            self.block.checked_add(block.try_into().ok()?)?,
            if block == 0 {
                self.tx.checked_add(tx.try_into().ok()?)?
            } else {
                tx.try_into().ok()?
            },
        ))
    }
}

impl Display for RuneId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.block, self.tx)
    }
}

impl BorshSerialize for RuneId {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        borsh::BorshSerialize::serialize(&self.block, writer)?;
        borsh::BorshSerialize::serialize(&self.tx, writer)?;
        Ok(())
    }
}

impl BorshDeserialize for RuneId {
    fn deserialize(buf: &mut &[u8]) -> std::io::Result<Self> {
        let block = <u64 as borsh::BorshDeserialize>::deserialize(buf)?;
        let tx = <u32 as borsh::BorshDeserialize>::deserialize(buf)?;
        Ok(RuneId::new(block, tx))
    }

    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        let block = u64::deserialize_reader(reader)?;
        let tx = u32::deserialize_reader(reader)?;
        Ok(RuneId::new(block, tx))
    }
}

impl FromStr for RuneId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (height, index) = s
            .split_once(':')
            .ok_or_else(|| "Invalid format: expected 'block:tx'".to_string())?;

        let block = height
            .parse::<u64>()
            .map_err(|_| "Invalid block number".to_string())?;
        let tx = index
            .parse::<u32>()
            .map_err(|_| "Invalid transaction number".to_string())?;

        Ok(RuneId::new(block, tx))
    }
}

impl Serialize for RuneId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for RuneId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = <String as serde::Deserialize>::deserialize(deserializer)?;
        let rune_id = RuneId::from_str(&s).map_err(serde::de::Error::custom)?;
        Ok(rune_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta() {
        let mut expected = [
            RuneId::new(3, 1),
            RuneId::new(4, 2),
            RuneId::new(1, 2),
            RuneId::new(1, 1),
            RuneId::new(3, 1),
            RuneId::new(2, 0),
        ];

        expected.sort();

        assert_eq!(
            expected,
            [
                RuneId::new(1, 1),
                RuneId::new(1, 2),
                RuneId::new(2, 0),
                RuneId::new(3, 1),
                RuneId::new(3, 1),
                RuneId::new(4, 2),
            ]
        );

        let mut previous = RuneId::default();
        let mut deltas = Vec::new();
        for id in expected {
            deltas.push(previous.delta(id).unwrap());
            previous = id;
        }

        assert_eq!(deltas, [(1, 1), (0, 1), (1, 0), (1, 1), (0, 0), (1, 2)]);

        let mut previous = RuneId::default();
        let mut actual = Vec::new();
        for (block, tx) in deltas {
            let next = previous.next(block, tx).unwrap();
            actual.push(next);
            previous = next;
        }

        assert_eq!(actual, expected);
    }

    #[test]
    fn display() {
        assert_eq!(RuneId::new(1, 2).to_string(), "1:2");
    }

    #[test]
    fn from_str() {
        assert!("123".parse::<RuneId>().is_err());
        assert!(":".parse::<RuneId>().is_err());
        assert!("1:".parse::<RuneId>().is_err());
        assert!(":2".parse::<RuneId>().is_err());
        assert!("a:2".parse::<RuneId>().is_err());
        assert!("1:a".parse::<RuneId>().is_err());
        assert_eq!("1:2".parse::<RuneId>().unwrap(), RuneId::new(1, 2));
        // block == 0 && tx > 0 is now valid
        assert_eq!("0:1".parse::<RuneId>().unwrap(), RuneId::new(0, 1));
    }

    #[test]
    fn serde() {
        let rune_id = RuneId::new(1, 2);
        let json = "\"1:2\"";
        assert_eq!(serde_json::to_string(&rune_id).unwrap(), json);
        assert_eq!(serde_json::from_str::<RuneId>(json).unwrap(), rune_id);
    }

    #[test]
    fn to_bytes() {
        let rune_id = RuneId::new(0x1234567890ABCDEF, 0x12345678);
        let bytes = rune_id.to_bytes();

        // Check little-endian encoding
        assert_eq!(bytes[0..8], 0x1234567890ABCDEFu64.to_le_bytes());
        assert_eq!(bytes[8..12], 0x12345678u32.to_le_bytes());
    }

    #[test]
    fn get_sorted_rune_ids() {
        let rune1 = RuneId::new(1, 1);
        let rune2 = RuneId::new(2, 2);

        let (a, b) = RuneId::get_sorted_rune_ids(&rune1, &rune2);
        assert!(a <= b);

        // Should be same regardless of order
        let (c, d) = RuneId::get_sorted_rune_ids(&rune2, &rune1);
        assert_eq!((a, b), (c, d));
    }

    #[test]
    fn borsh_roundtrip() {
        let rune_id = RuneId::new(840000, 1);
        let serialized = borsh::to_vec(&rune_id).unwrap();
        let deserialized: RuneId = borsh::from_slice(&serialized).unwrap();
        assert_eq!(rune_id, deserialized);
    }
}
