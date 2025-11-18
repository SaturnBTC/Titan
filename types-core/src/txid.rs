use bitcoin::{hashes::Hash, Txid};
use std::{fmt::Display, str::FromStr};

#[cfg(feature = "borsh")]
use borsh::{BorshDeserialize, BorshSerialize};
#[cfg(feature = "borsh")]
use std::io::{Read, Result, Write};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(PartialEq, Eq, Hash, Clone, Copy)]
pub struct SerializedTxid(pub [u8; 32]);

impl SerializedTxid {
    pub fn new(txid: &[u8]) -> Self {
        Self(txid.try_into().unwrap())
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn all_zeros() -> Self {
        Self([0; 32])
    }
}

impl AsRef<[u8]> for SerializedTxid {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl From<[u8; 32]> for SerializedTxid {
    fn from(txid: [u8; 32]) -> Self {
        Self(txid)
    }
}

impl From<&[u8]> for SerializedTxid {
    fn from(txid: &[u8]) -> Self {
        Self(txid.try_into().unwrap())
    }
}

impl TryFrom<Box<[u8]>> for SerializedTxid {
    type Error = std::array::TryFromSliceError;

    fn try_from(txid: Box<[u8]>) -> std::result::Result<Self, Self::Error> {
        let array: [u8; 32] = (*txid).try_into()?;
        Ok(Self(array))
    }
}

impl From<Txid> for SerializedTxid {
    fn from(txid: Txid) -> Self {
        Self(*txid.as_raw_hash().as_byte_array())
    }
}

impl From<&Txid> for SerializedTxid {
    fn from(txid: &Txid) -> Self {
        Self(*txid.as_raw_hash().as_byte_array())
    }
}

impl From<&&Txid> for SerializedTxid {
    fn from(txid: &&Txid) -> Self {
        Self(*txid.as_raw_hash().as_byte_array())
    }
}

impl From<SerializedTxid> for Txid {
    fn from(txid: SerializedTxid) -> Self {
        Self::from_raw_hash(Hash::from_slice(&txid.0).unwrap())
    }
}

impl From<&SerializedTxid> for Txid {
    fn from(txid: &SerializedTxid) -> Self {
        Self::from_raw_hash(Hash::from_slice(&txid.0).unwrap())
    }
}

impl std::fmt::Debug for SerializedTxid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_string())
    }
}

impl Display for SerializedTxid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Display as hex string (reversed bytes like bitcoin::Txid)
        let mut reversed = self.0;
        reversed.reverse();
        write!(f, "{}", hex::encode(reversed))
    }
}

impl FromStr for SerializedTxid {
    type Err = hex::FromHexError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        // Parse hex string and reverse bytes (bitcoin txids are displayed in reverse order)
        let mut bytes = hex::decode(s)?;
        if bytes.len() != 32 {
            return Err(hex::FromHexError::InvalidStringLength);
        }
        bytes.reverse();
        Ok(Self(bytes.try_into().unwrap()))
    }
}

#[cfg(feature = "borsh")]
impl BorshSerialize for SerializedTxid {
    fn serialize<W: Write>(&self, writer: &mut W) -> Result<()> {
        // Serialize the raw 32 bytes
        BorshSerialize::serialize(&self.0, writer)
    }
}

#[cfg(feature = "borsh")]
impl BorshDeserialize for SerializedTxid {
    fn deserialize_reader<R: Read>(reader: &mut R) -> Result<Self> {
        let bytes = <[u8; 32]>::deserialize_reader(reader)?;
        Ok(Self(bytes))
    }
}

#[cfg(feature = "serde")]
impl Serialize for SerializedTxid {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Serialize as hex string with reversed bytes (same as bitcoin::Txid)
        let mut reversed = self.0;
        reversed.reverse();
        let hex_string = hex::encode(reversed);
        serializer.serialize_str(&hex_string)
    }
}

#[cfg(feature = "serde")]
impl<'de> Deserialize<'de> for SerializedTxid {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let hex_string = <String as serde::Deserialize>::deserialize(deserializer)?;
        Self::from_str(&hex_string).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::{hashes::Hash, Txid};
    #[cfg(feature = "borsh")]
    use borsh::{BorshDeserialize, BorshSerialize};

    /// Helper function to test borsh serialization roundtrip
    fn test_borsh_roundtrip<T>(original: &T) -> T
    where
        T: BorshSerialize + BorshDeserialize + std::fmt::Debug + PartialEq,
    {
        let serialized = borsh::to_vec(original).expect("Failed to serialize");
        let deserialized = borsh::from_slice(&serialized).expect("Failed to deserialize");
        assert_eq!(original, &deserialized, "Borsh roundtrip failed");
        deserialized
    }

    /// Helper function to test serde serialization roundtrip
    fn test_serde_roundtrip<T>(original: &T) -> T
    where
        T: serde::Serialize + for<'de> serde::Deserialize<'de> + std::fmt::Debug + PartialEq,
    {
        let serialized = serde_json::to_string(original).expect("Failed to serialize");
        let deserialized = serde_json::from_str(&serialized).expect("Failed to deserialize");
        assert_eq!(original, &deserialized, "Serde roundtrip failed");
        deserialized
    }

    fn create_test_txid() -> SerializedTxid {
        SerializedTxid::from([42u8; 32])
    }

    fn create_zero_txid() -> SerializedTxid {
        SerializedTxid::all_zeros()
    }

    fn create_max_txid() -> SerializedTxid {
        SerializedTxid::from([255u8; 32])
    }

    #[test]
    fn test_serialized_txid_new() {
        let bytes = [42u8; 32];
        let txid = SerializedTxid::new(&bytes);
        assert_eq!(txid.as_bytes(), &bytes);
    }

    #[test]
    fn test_serialized_txid_borsh_roundtrip() {
        let original = create_test_txid();
        test_borsh_roundtrip(&original);
    }

    #[test]
    fn test_serialized_txid_serde_roundtrip() {
        let original = create_test_txid();
        test_serde_roundtrip(&original);
    }

    #[test]
    fn test_serialized_txid_zero_values() {
        let original = create_zero_txid();
        test_borsh_roundtrip(&original);
        test_serde_roundtrip(&original);
    }

    #[test]
    fn test_serialized_txid_max_values() {
        let original = create_max_txid();
        test_borsh_roundtrip(&original);
        test_serde_roundtrip(&original);
    }

    #[test]
    fn test_serialized_txid_all_zeros() {
        let txid = SerializedTxid::all_zeros();
        assert_eq!(txid.as_bytes(), &[0u8; 32]);
    }

    #[test]
    fn test_serialized_txid_as_bytes() {
        let bytes = [123u8; 32];
        let txid = SerializedTxid::from(bytes);
        assert_eq!(txid.as_bytes(), &bytes);
    }

    #[test]
    fn test_serialized_txid_as_ref() {
        let bytes = [99u8; 32];
        let txid = SerializedTxid::from(bytes);
        let as_ref: &[u8] = txid.as_ref();
        assert_eq!(as_ref, &bytes);
    }

    #[test]
    fn test_serialized_txid_from_array() {
        let array = [77u8; 32];
        let txid = SerializedTxid::from(array);
        assert_eq!(txid.as_bytes(), &array);
    }

    #[test]
    fn test_serialized_txid_from_slice() {
        let array = [88u8; 32];
        let slice = array.as_slice();
        let txid = SerializedTxid::from(slice);
        assert_eq!(txid.as_bytes(), &array);
    }

    #[test]
    fn test_serialized_txid_try_from_box() {
        let boxed: Box<[u8]> = Box::new([44u8; 32]);
        let txid = SerializedTxid::try_from(boxed).expect("Should convert from Box<[u8]>");
        assert_eq!(txid.as_bytes(), &[44u8; 32]);
    }

    #[test]
    fn test_serialized_txid_try_from_box_invalid_length() {
        let boxed: Box<[u8]> = Box::new([44u8; 31]); // Wrong length
        let result = SerializedTxid::try_from(boxed);
        assert!(result.is_err(), "Should fail with invalid length");
    }

    #[test]
    fn test_serialized_txid_from_bitcoin_txid() {
        let bitcoin_txid = Txid::from_raw_hash(Hash::from_slice(&[100u8; 32]).unwrap());
        let serialized_txid = SerializedTxid::from(bitcoin_txid);
        assert_eq!(
            serialized_txid.as_bytes(),
            bitcoin_txid.as_raw_hash().as_byte_array()
        );
    }

    #[test]
    fn test_serialized_txid_from_bitcoin_txid_ref() {
        let bitcoin_txid = Txid::from_raw_hash(Hash::from_slice(&[101u8; 32]).unwrap());
        let serialized_txid = SerializedTxid::from(&bitcoin_txid);
        assert_eq!(
            serialized_txid.as_bytes(),
            bitcoin_txid.as_raw_hash().as_byte_array()
        );
    }

    #[test]
    fn test_serialized_txid_from_bitcoin_txid_double_ref() {
        let bitcoin_txid = Txid::from_raw_hash(Hash::from_slice(&[102u8; 32]).unwrap());
        let serialized_txid = SerializedTxid::from(&&bitcoin_txid);
        assert_eq!(
            serialized_txid.as_bytes(),
            bitcoin_txid.as_raw_hash().as_byte_array()
        );
    }

    #[test]
    fn test_serialized_txid_to_bitcoin_txid() {
        let bytes = [103u8; 32];
        let serialized_txid = SerializedTxid::from(bytes);
        let bitcoin_txid: Txid = serialized_txid.into();
        assert_eq!(bitcoin_txid.as_raw_hash().as_byte_array(), &bytes);
    }

    #[test]
    fn test_serialized_txid_to_bitcoin_txid_ref() {
        let bytes = [104u8; 32];
        let serialized_txid = SerializedTxid::from(bytes);
        let bitcoin_txid: Txid = (&serialized_txid).into();
        assert_eq!(bitcoin_txid.as_raw_hash().as_byte_array(), &bytes);
    }

    #[test]
    fn test_serialized_txid_display() {
        let bytes = [1u8; 32];
        let txid = SerializedTxid::from(bytes);
        let display_str = txid.to_string();

        // The display should show the reversed hex string (Bitcoin convention)
        assert_eq!(display_str.len(), 64); // 32 bytes * 2 hex chars
        assert!(display_str.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_serialized_txid_from_str() {
        // Test with a known pattern
        let hex_str = "0101010101010101010101010101010101010101010101010101010101010101";
        let txid = SerializedTxid::from_str(hex_str).expect("Should parse valid hex string");

        // Verify roundtrip
        let back_to_string = txid.to_string();
        assert_eq!(hex_str, back_to_string);
    }

    #[test]
    fn test_serialized_txid_from_str_invalid_length() {
        let invalid_hex = "0101"; // Too short
        let result = SerializedTxid::from_str(invalid_hex);
        assert!(result.is_err(), "Should fail with invalid length");
    }

    #[test]
    fn test_serialized_txid_from_str_invalid_hex() {
        let invalid_hex = "gggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggg";
        let result = SerializedTxid::from_str(invalid_hex);
        assert!(result.is_err(), "Should fail with invalid hex characters");
    }

    #[test]
    fn test_serialized_txid_consistency() {
        let original = create_test_txid();

        // Test that multiple serializations produce the same result
        let serialized1 = borsh::to_vec(&original).unwrap();
        let serialized2 = borsh::to_vec(&original).unwrap();
        assert_eq!(serialized1, serialized2);

        // Test that deserialization produces the same value
        let deserialized1 = borsh::from_slice::<SerializedTxid>(&serialized1).unwrap();
        let deserialized2 = borsh::from_slice::<SerializedTxid>(&serialized2).unwrap();
        assert_eq!(deserialized1, deserialized2);
        assert_eq!(original, deserialized1);
    }

    #[test]
    fn test_serialized_txid_hash_and_eq() {
        let txid1 = create_test_txid();
        let txid2 = create_test_txid();
        let txid3 = create_zero_txid();

        assert_eq!(txid1, txid2);
        assert_ne!(txid1, txid3);

        // Test that equal values have the same hash
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher1 = DefaultHasher::new();
        let mut hasher2 = DefaultHasher::new();
        txid1.hash(&mut hasher1);
        txid2.hash(&mut hasher2);
        assert_eq!(hasher1.finish(), hasher2.finish());
    }

    #[test]
    fn test_serialized_txid_clone_copy() {
        let original = create_test_txid();
        let cloned = original.clone();
        let copied = original;

        assert_eq!(original, cloned);
        assert_eq!(original, copied);
    }

    #[test]
    fn test_serialized_txid_debug() {
        let txid = create_test_txid();
        let debug_str = format!("{:?}", txid);
        assert!(debug_str.contains("SerializedTxid"));
    }

    #[test]
    fn test_serialized_txid_patterns() {
        // Test various bit patterns
        let patterns = [
            [0u8; 32],
            [255u8; 32],
            {
                let mut pattern = [0u8; 32];
                pattern[0] = 255;
                pattern
            },
            {
                let mut pattern = [0u8; 32];
                pattern[31] = 255;
                pattern
            },
            {
                let mut pattern = [0u8; 32];
                for i in 0..32 {
                    pattern[i] = i as u8;
                }
                pattern
            },
        ];

        for pattern in patterns {
            let txid = SerializedTxid::from(pattern);
            test_borsh_roundtrip(&txid);
            test_serde_roundtrip(&txid);

            // Test string roundtrip
            let hex_str = txid.to_string();
            let from_str = SerializedTxid::from_str(&hex_str).unwrap();
            assert_eq!(txid, from_str);
        }
    }

    #[test]
    fn test_serialized_txid_bitcoin_interop() {
        // Test interoperability with Bitcoin txids
        let test_cases = [[0u8; 32], [1u8; 32], [255u8; 32]];

        for bytes in test_cases {
            let serialized_txid = SerializedTxid::from(bytes);

            // Convert to Bitcoin Txid and back
            let bitcoin_txid: Txid = serialized_txid.into();
            let back_to_serialized: SerializedTxid = bitcoin_txid.into();

            assert_eq!(serialized_txid, back_to_serialized);
        }
    }
}
