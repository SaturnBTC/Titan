use std::io::{Read, Result, Write};

use borsh::{BorshDeserialize, BorshSerialize};
use ordinals::{Rune, SpacedRune, Terms};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use titan_types::{InscriptionId, MintResponse, RuneId, RuneResponse, SerializedTxid};

#[derive(Debug, PartialEq, Error)]
pub enum MintError {
    #[error("rune is not mintable")]
    Unmintable,
    #[error("rune is not mintable before start height {0}")]
    Start(u64),
    #[error("rune is not mintable after end height {0}")]
    End(u64),
    #[error("rune has reached cap {0}")]
    Cap(u128),
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct RuneEntry {
    pub block: u64,
    pub burned: u128,
    pub divisibility: u8,
    pub etching: SerializedTxid,
    pub mints: u128,
    pub number: u64,
    pub premine: u128,
    pub spaced_rune: SpacedRune,
    pub symbol: Option<char>,
    pub terms: Option<Terms>,
    pub pending_burns: u128,
    pub pending_mints: u128,
    pub inscription_id: Option<InscriptionId>,
    pub timestamp: u64,
    pub turbo: bool,
}

impl RuneEntry {
    pub fn mintable(&self, height: u64) -> std::result::Result<u128, MintError> {
        let Some(terms) = self.terms else {
            return Err(MintError::Unmintable);
        };

        if let Some(start) = self.start() {
            if height < start {
                return Err(MintError::Start(start));
            }
        }

        if let Some(end) = self.end() {
            if height >= end {
                return Err(MintError::End(end));
            }
        }

        let cap = terms.cap.unwrap_or_default();

        if self.mints >= cap {
            return Err(MintError::Cap(cap));
        }

        Ok(terms.amount.unwrap_or_default())
    }

    pub fn supply(&self) -> u128 {
        self.premine
            + self.mints
                * self
                    .terms
                    .and_then(|terms| terms.amount)
                    .unwrap_or_default()
    }

    pub fn max_supply(&self) -> u128 {
        self.premine
            + self.terms.and_then(|terms| terms.cap).unwrap_or_default()
                * self
                    .terms
                    .and_then(|terms| terms.amount)
                    .unwrap_or_default()
    }

    pub fn start(&self) -> Option<u64> {
        let terms = self.terms?;

        let relative = terms
            .offset
            .0
            .map(|offset| self.block.saturating_add(offset));

        let absolute = terms.height.0;

        relative
            .zip(absolute)
            .map(|(relative, absolute)| relative.max(absolute))
            .or(relative)
            .or(absolute)
    }

    pub fn end(&self) -> Option<u64> {
        let terms = self.terms?;

        let relative = terms
            .offset
            .1
            .map(|offset| self.block.saturating_add(offset));

        let absolute = terms.height.1;

        relative
            .zip(absolute)
            .map(|(relative, absolute)| relative.min(absolute))
            .or(relative)
            .or(absolute)
    }

    pub fn to_rune_response(&self, id: RuneId, height: u64) -> RuneResponse {
        let mintable = match self.mintable(height) {
            Ok(_) => true,
            Err(_) => false,
        };

        let mint = if let Some(terms) = self.terms {
            Some(MintResponse {
                start: self.start(),
                end: self.end(),
                mintable,
                cap: terms.cap.unwrap_or_default(),
                amount: terms.amount.unwrap_or_default(),
                mints: self.mints,
            })
        } else {
            None
        };

        RuneResponse {
            id,
            block: self.block,
            burned: self.burned,
            divisibility: self.divisibility,
            etching: self.etching,
            number: self.number,
            premine: self.premine,
            supply: self.supply(),
            max_supply: self.max_supply(),
            spaced_rune: self.spaced_rune,
            symbol: self.symbol,
            mint,
            burns: self.pending_burns,
            pending_burns: self.pending_burns,
            pending_mints: self.pending_mints,
            inscription_id: self.inscription_id.clone(),
            timestamp: self.timestamp,
            turbo: self.turbo,
        }
    }
}

impl BorshSerialize for RuneEntry {
    fn serialize<W: Write>(&self, writer: &mut W) -> Result<()> {
        BorshSerialize::serialize(&self.block, writer)?;
        BorshSerialize::serialize(&self.burned, writer)?;
        BorshSerialize::serialize(&self.divisibility, writer)?;
        BorshSerialize::serialize(&self.etching, writer)?;
        BorshSerialize::serialize(&self.mints, writer)?;
        BorshSerialize::serialize(&self.number, writer)?;
        BorshSerialize::serialize(&self.premine, writer)?;

        // Now handle your `spaced_rune` field manually:
        //
        //  spaced_rune: SpacedRune {
        //    rune: Rune(pub u128),
        //    spacers: u32
        //  }
        //
        // Just write `rune.0` and `spacers`.
        let rune_u128 = (self.spaced_rune.rune).0;
        BorshSerialize::serialize(&rune_u128, writer)?;
        BorshSerialize::serialize(&self.spaced_rune.spacers, writer)?;

        // Next, handle `symbol`:
        // Option<char> is not directly Borsh-friendly, so
        // typically store as Option<u32> or something.
        match self.symbol {
            Some(ch) => {
                BorshSerialize::serialize(&true, writer)?; // indicating Some
                BorshSerialize::serialize(&(ch as u32), writer)?; // store as u32
            }
            None => {
                BorshSerialize::serialize(&false, writer)?; // indicating None
            }
        }

        // Next, handle `terms`: Option<Terms>
        // If Terms also doesn’t implement Borsh, do it manually:
        //   Terms {
        //       amount: Option<u128>,
        //       cap: Option<u128>,
        //       height: (Option<u64>, Option<u64>),
        //       offset: (Option<u64>, Option<u64>),
        //   }
        match &self.terms {
            Some(terms) => {
                BorshSerialize::serialize(&true, writer)?; // indicates Some(Terms)
                                                           // Now each Terms field in turn
                                                           // -- amount
                match terms.amount {
                    Some(a) => {
                        BorshSerialize::serialize(&true, writer)?;
                        BorshSerialize::serialize(&a, writer)?;
                    }
                    None => {
                        BorshSerialize::serialize(&false, writer)?;
                    }
                }
                // -- cap
                match terms.cap {
                    Some(c) => {
                        BorshSerialize::serialize(&true, writer)?;
                        BorshSerialize::serialize(&c, writer)?;
                    }
                    None => {
                        BorshSerialize::serialize(&false, writer)?;
                    }
                }
                // -- height
                //    (Option<u64>, Option<u64>)
                let (h1, h2) = terms.height;
                match h1 {
                    Some(h) => {
                        BorshSerialize::serialize(&true, writer)?;
                        BorshSerialize::serialize(&h, writer)?;
                    }
                    None => {
                        BorshSerialize::serialize(&false, writer)?;
                    }
                }
                match h2 {
                    Some(h) => {
                        BorshSerialize::serialize(&true, writer)?;
                        BorshSerialize::serialize(&h, writer)?;
                    }
                    None => {
                        BorshSerialize::serialize(&false, writer)?;
                    }
                }
                // -- offset
                let (o1, o2) = terms.offset;
                match o1 {
                    Some(o) => {
                        BorshSerialize::serialize(&true, writer)?;
                        BorshSerialize::serialize(&o, writer)?;
                    }
                    None => {
                        BorshSerialize::serialize(&false, writer)?;
                    }
                }
                match o2 {
                    Some(o) => {
                        BorshSerialize::serialize(&true, writer)?;
                        BorshSerialize::serialize(&o, writer)?;
                    }
                    None => {
                        BorshSerialize::serialize(&false, writer)?;
                    }
                }
            }
            None => {
                BorshSerialize::serialize(&false, writer)?; // indicates None
            }
        }

        match &self.inscription_id {
            Some(inscription_id) => {
                BorshSerialize::serialize(&true, writer)?;
                BorshSerialize::serialize(&inscription_id, writer)?;
            }
            None => {
                BorshSerialize::serialize(&false, writer)?;
            }
        }

        BorshSerialize::serialize(&self.pending_burns, writer)?;
        BorshSerialize::serialize(&self.pending_mints, writer)?;
        BorshSerialize::serialize(&self.timestamp, writer)?;
        BorshSerialize::serialize(&self.turbo, writer)?;

        Ok(())
    }
}

impl BorshDeserialize for RuneEntry {
    fn deserialize_reader<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        // Read back in the same order:
        let block = u64::deserialize_reader(reader)?;
        let burned = u128::deserialize_reader(reader)?;
        let divisibility = u8::deserialize_reader(reader)?;
        let etching = SerializedTxid::deserialize_reader(reader)?;
        let mints = u128::deserialize_reader(reader)?;
        let number = u64::deserialize_reader(reader)?;
        let premine = u128::deserialize_reader(reader)?;
        // spaced_rune:
        let rune_u128 = u128::deserialize_reader(reader)?;
        let spacers = u32::deserialize_reader(reader)?;
        let spaced_rune = SpacedRune {
            rune: Rune(rune_u128),
            spacers,
        };

        // symbol: Option<char> stored as bool + u32
        let symbol_present = bool::deserialize_reader(reader)?;
        let symbol = if symbol_present {
            let val = u32::deserialize_reader(reader)?;
            Some(std::char::from_u32(val).ok_or(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Invalid char code: {}", val),
            ))?)
        } else {
            None
        };

        // terms: Option<Terms>
        let terms_present = bool::deserialize_reader(reader)?;
        let terms = if terms_present {
            // amount
            let amount_present = bool::deserialize_reader(reader)?;
            let amount = if amount_present {
                Some(u128::deserialize_reader(reader)?)
            } else {
                None
            };
            // cap
            let cap_present = bool::deserialize_reader(reader)?;
            let cap = if cap_present {
                Some(u128::deserialize_reader(reader)?)
            } else {
                None
            };
            // height
            let h1_present = bool::deserialize_reader(reader)?;
            let h1 = if h1_present {
                Some(u64::deserialize_reader(reader)?)
            } else {
                None
            };
            let h2_present = bool::deserialize_reader(reader)?;
            let h2 = if h2_present {
                Some(u64::deserialize_reader(reader)?)
            } else {
                None
            };
            let height = (h1, h2);

            // offset
            let o1_present = bool::deserialize_reader(reader)?;
            let o1 = if o1_present {
                Some(u64::deserialize_reader(reader)?)
            } else {
                None
            };
            let o2_present = bool::deserialize_reader(reader)?;
            let o2 = if o2_present {
                Some(u64::deserialize_reader(reader)?)
            } else {
                None
            };
            let offset = (o1, o2);

            Some(Terms {
                amount,
                cap,
                height,
                offset,
            })
        } else {
            None
        };

        // inscription_id: Option<InscriptionId>
        let inscription_id_present = bool::deserialize_reader(reader)?;
        let inscription_id = if inscription_id_present {
            Some(InscriptionId::deserialize_reader(reader)?)
        } else {
            None
        };

        let pending_burns = u128::deserialize_reader(reader)?;
        let pending_mints = u128::deserialize_reader(reader)?;
        let timestamp = u64::deserialize_reader(reader)?;
        let turbo = bool::deserialize_reader(reader)?;

        Ok(Self {
            block,
            burned,
            divisibility,
            etching,
            mints,
            number,
            premine,
            spaced_rune,
            symbol,
            terms,
            pending_burns,
            pending_mints,
            inscription_id,
            timestamp,
            turbo,
        })
    }
}
