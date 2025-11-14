use {
    crate::models::{BlockId, Inscription, RuneEntry, TransactionStateChange, TxRuneIndexRef},
    borsh::{BorshDeserialize, BorshSerialize},
    titan_types::{Block, MempoolEntry, RuneId, SpenderReference, Subscription, TxOut},
};

pub trait Entry: Sized + BorshDeserialize + BorshSerialize {
    fn load(value: Vec<u8>) -> Self {
        BorshDeserialize::deserialize(&mut &value[..]).unwrap()
    }

    fn store(self) -> Vec<u8> {
        let mut serialized = Vec::new();
        self.serialize(&mut serialized).unwrap();
        serialized
    }

    /// Serialize the entry by reference without consuming `self`.
    fn store_ref(&self) -> Vec<u8> {
        let mut serialized = Vec::new();
        self.serialize(&mut serialized).unwrap();
        serialized
    }
}

impl Entry for Block {}
impl Entry for BlockId {}
impl Entry for Inscription {}
impl Entry for RuneEntry {}
impl Entry for TxRuneIndexRef {}
impl Entry for Vec<TxRuneIndexRef> {}
impl Entry for TransactionStateChange {}
impl Entry for TxOut {}
impl Entry for Subscription {}
impl Entry for SpenderReference {}
impl Entry for MempoolEntry {}
impl Entry for RuneId {}
