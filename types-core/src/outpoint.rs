use std::str::FromStr;

use bitcoin::{hashes::Hash, OutPoint, Txid};
#[cfg(feature = "borsh")]
use borsh::{BorshDeserialize, BorshSerialize};
#[cfg(feature = "serde")]
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::SerializedTxid;

#[derive(PartialEq, Eq, Hash, Clone, Copy)]
#[cfg_attr(feature = "borsh", derive(BorshSerialize, BorshDeserialize))]
pub struct SerializedOutPoint([u8; 36]);

impl SerializedOutPoint {
    pub fn new(txid: &[u8], vout: u32) -> Self {
        let mut outpoint = [0u8; 36];
        outpoint[..32].copy_from_slice(txid);
        outpoint[32..].copy_from_slice(&vout.to_le_bytes());
        Self(outpoint)
    }

    pub fn from_txid_vout(txid: &SerializedTxid, vout: u32) -> Self {
        Self::new(txid.as_ref(), vout)
    }

    pub fn txid(&self) -> &[u8] {
        &self.0[..32]
    }

    pub fn to_txid(&self) -> Txid {
        Txid::from_raw_hash(Hash::from_slice(self.txid()).unwrap())
    }

    pub fn to_serialized_txid(&self) -> SerializedTxid {
        SerializedTxid::from(self.txid())
    }

    pub fn vout(&self) -> u32 {
        u32::from_le_bytes(self.0[32..].try_into().unwrap())
    }
}

impl AsRef<[u8]> for SerializedOutPoint {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl From<SerializedOutPoint> for OutPoint {
    fn from(outpoint: SerializedOutPoint) -> Self {
        OutPoint::new(outpoint.to_txid(), outpoint.vout())
    }
}

impl From<OutPoint> for SerializedOutPoint {
    fn from(outpoint: OutPoint) -> Self {
        Self::new(outpoint.txid.as_raw_hash().as_byte_array(), outpoint.vout)
    }
}

impl From<[u8; 36]> for SerializedOutPoint {
    fn from(outpoint: [u8; 36]) -> Self {
        Self(outpoint)
    }
}

impl From<&[u8]> for SerializedOutPoint {
    fn from(outpoint: &[u8]) -> Self {
        Self(outpoint.try_into().unwrap())
    }
}

impl TryFrom<Box<[u8]>> for SerializedOutPoint {
    type Error = std::array::TryFromSliceError;

    fn try_from(outpoint: Box<[u8]>) -> Result<Self, Self::Error> {
        let array: [u8; 36] = outpoint.as_ref().try_into()?;
        Ok(Self(array))
    }
}

impl std::fmt::Debug for SerializedOutPoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.to_txid(), self.vout())
    }
}

impl std::fmt::Display for SerializedOutPoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.to_txid(), self.vout())
    }
}

impl FromStr for SerializedOutPoint {
    type Err = std::io::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split(':').collect();
        Ok(Self::from_txid_vout(
            &SerializedTxid::from_str(parts[0]).map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "Invalid txid")
            })?,
            parts[1].parse().map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "Invalid vout")
            })?,
        ))
    }
}

#[cfg(feature = "serde")]
impl Serialize for SerializedOutPoint {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Serialize as a struct with txid and vout fields like OutPoint
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("SerializedOutPoint", 2)?;
        state.serialize_field("txid", &self.to_txid())?;
        state.serialize_field("vout", &self.vout())?;
        state.end()
    }
}

#[cfg(feature = "serde")]
impl<'de> Deserialize<'de> for SerializedOutPoint {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::{self, MapAccess, Visitor};
        use std::fmt;

        #[derive(serde::Deserialize)]
        #[serde(field_identifier, rename_all = "lowercase")]
        enum Field {
            Txid,
            Vout,
        }

        struct SerializedOutPointVisitor;

        impl<'de> Visitor<'de> for SerializedOutPointVisitor {
            type Value = SerializedOutPoint;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("struct SerializedOutPoint")
            }

            fn visit_map<V>(self, mut map: V) -> Result<SerializedOutPoint, V::Error>
            where
                V: MapAccess<'de>,
            {
                let mut txid = None;
                let mut vout = None;
                while let Some(key) = map.next_key()? {
                    match key {
                        Field::Txid => {
                            if txid.is_some() {
                                return Err(de::Error::duplicate_field("txid"));
                            }
                            txid = Some(map.next_value()?);
                        }
                        Field::Vout => {
                            if vout.is_some() {
                                return Err(de::Error::duplicate_field("vout"));
                            }
                            vout = Some(map.next_value()?);
                        }
                    }
                }
                let txid: SerializedTxid = txid.ok_or_else(|| de::Error::missing_field("txid"))?;
                let vout: u32 = vout.ok_or_else(|| de::Error::missing_field("vout"))?;
                Ok(SerializedOutPoint::from_txid_vout(&txid, vout))
            }
        }

        const FIELDS: &'static [&'static str] = &["txid", "vout"];
        deserializer.deserialize_struct("SerializedOutPoint", FIELDS, SerializedOutPointVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::{hashes::Hash, OutPoint};
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

    fn create_test_outpoint() -> SerializedOutPoint {
        SerializedOutPoint::new(&[42u8; 32], 12345)
    }

    fn create_zero_outpoint() -> SerializedOutPoint {
        SerializedOutPoint::new(&[0u8; 32], 0)
    }

    fn create_max_outpoint() -> SerializedOutPoint {
        SerializedOutPoint::new(&[255u8; 32], u32::MAX)
    }

    #[test]
    fn test_serialized_outpoint_creation() {
        let txid_bytes = [42u8; 32];
        let vout = 12345;
        let outpoint = SerializedOutPoint::new(&txid_bytes, vout);

        assert_eq!(outpoint.txid(), &txid_bytes);
        assert_eq!(outpoint.vout(), vout);
    }

    #[test]
    fn test_serialized_outpoint_from_txid_vout() {
        let txid = SerializedTxid::from([42u8; 32]);
        let vout = 12345;
        let outpoint = SerializedOutPoint::from_txid_vout(&txid, vout);

        assert_eq!(outpoint.txid(), txid.as_bytes());
        assert_eq!(outpoint.vout(), vout);
    }

    #[test]
    fn test_serialized_outpoint_borsh_roundtrip() {
        let original = create_test_outpoint();
        test_borsh_roundtrip(&original);
    }

    #[test]
    fn test_serialized_outpoint_serde_roundtrip() {
        let original = create_test_outpoint();
        test_serde_roundtrip(&original);
    }

    #[test]
    fn test_serialized_outpoint_zero_values() {
        let original = create_zero_outpoint();
        test_borsh_roundtrip(&original);
        test_serde_roundtrip(&original);
    }

    #[test]
    fn test_serialized_outpoint_max_values() {
        let original = create_max_outpoint();
        test_borsh_roundtrip(&original);
        test_serde_roundtrip(&original);
    }

    #[test]
    fn test_serialized_outpoint_conversions() {
        let original_bytes = [42u8; 32];
        let vout = 12345;
        let serialized_outpoint = SerializedOutPoint::new(&original_bytes, vout);

        // Test conversion to Bitcoin OutPoint
        let bitcoin_outpoint: OutPoint = serialized_outpoint.into();
        assert_eq!(bitcoin_outpoint.vout, vout);

        // Test conversion back from Bitcoin OutPoint
        let converted_back: SerializedOutPoint = bitcoin_outpoint.into();
        assert_eq!(converted_back.vout(), vout);
        assert_eq!(converted_back.txid(), &original_bytes);
    }

    #[test]
    fn test_serialized_outpoint_display() {
        let txid_bytes = [1u8; 32];
        let vout = 42;
        let outpoint = SerializedOutPoint::new(&txid_bytes, vout);

        let display_str = outpoint.to_string();
        assert!(display_str.contains(":"));
        assert!(display_str.contains(&vout.to_string()));
    }

    #[test]
    fn test_serialized_outpoint_from_str() {
        let txid_str = "1111111111111111111111111111111111111111111111111111111111111111";
        let vout = 42;
        let outpoint_str = format!("{}:{}", txid_str, vout);

        let outpoint = SerializedOutPoint::from_str(&outpoint_str)
            .expect("Should parse valid outpoint string");

        assert_eq!(outpoint.vout(), vout);
    }

    #[test]
    fn test_serialized_outpoint_from_array() {
        let mut array = [0u8; 36];
        array[..32].copy_from_slice(&[42u8; 32]);
        array[32..].copy_from_slice(&12345u32.to_le_bytes());

        let outpoint = SerializedOutPoint::from(array);
        assert_eq!(outpoint.txid(), &[42u8; 32]);
        assert_eq!(outpoint.vout(), 12345);
    }

    #[test]
    fn test_serialized_outpoint_from_slice() {
        let slice = &[42u8; 36];
        let outpoint = SerializedOutPoint::from(slice.as_slice());
        assert_eq!(outpoint.txid(), &[42u8; 32]);
        assert_eq!(outpoint.vout(), u32::from_le_bytes([42u8; 4]));
    }

    #[test]
    fn test_serialized_outpoint_try_from_box() {
        let boxed_slice: Box<[u8]> = Box::new([42u8; 36]);
        let outpoint =
            SerializedOutPoint::try_from(boxed_slice).expect("Should convert from Box<[u8]>");
        assert_eq!(outpoint.txid(), &[42u8; 32]);
    }

    #[test]
    fn test_serialized_outpoint_as_ref() {
        let outpoint = create_test_outpoint();
        let as_bytes: &[u8] = outpoint.as_ref();
        assert_eq!(as_bytes.len(), 36);
    }

    #[test]
    fn test_serialized_outpoint_to_txid() {
        let txid_bytes = [42u8; 32];
        let outpoint = SerializedOutPoint::new(&txid_bytes, 0);
        let bitcoin_txid = outpoint.to_txid();

        // Verify the txid conversion works correctly
        assert_eq!(bitcoin_txid.as_raw_hash().as_byte_array(), &txid_bytes);
    }

    #[test]
    fn test_serialized_outpoint_to_serialized_txid() {
        let txid_bytes = [42u8; 32];
        let outpoint = SerializedOutPoint::new(&txid_bytes, 0);
        let serialized_txid = outpoint.to_serialized_txid();

        assert_eq!(serialized_txid.as_bytes(), &txid_bytes);
    }

    #[test]
    fn test_serialized_outpoint_consistency() {
        let original = create_test_outpoint();

        // Test that multiple serializations produce the same result
        let serialized1 = borsh::to_vec(&original).unwrap();
        let serialized2 = borsh::to_vec(&original).unwrap();
        assert_eq!(serialized1, serialized2);

        // Test that deserialization produces the same value
        let deserialized1 = borsh::from_slice::<SerializedOutPoint>(&serialized1).unwrap();
        let deserialized2 = borsh::from_slice::<SerializedOutPoint>(&serialized2).unwrap();
        assert_eq!(deserialized1, deserialized2);
        assert_eq!(original, deserialized1);
    }

    #[test]
    fn test_serialized_outpoint_hash_and_eq() {
        let outpoint1 = create_test_outpoint();
        let outpoint2 = create_test_outpoint();
        let outpoint3 = create_zero_outpoint();

        assert_eq!(outpoint1, outpoint2);
        assert_ne!(outpoint1, outpoint3);

        // Test that equal values have the same hash
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher1 = DefaultHasher::new();
        let mut hasher2 = DefaultHasher::new();
        outpoint1.hash(&mut hasher1);
        outpoint2.hash(&mut hasher2);
        assert_eq!(hasher1.finish(), hasher2.finish());
    }

    #[test]
    fn test_serialized_outpoint_edge_cases() {
        // Test with different txid patterns
        let patterns = [[0u8; 32], [255u8; 32], {
            let mut pattern = [0u8; 32];
            pattern[0] = 255;
            pattern[31] = 255;
            pattern
        }];

        for pattern in patterns {
            let outpoint = SerializedOutPoint::new(&pattern, 42);
            test_borsh_roundtrip(&outpoint);
            test_serde_roundtrip(&outpoint);
        }
    }
}
