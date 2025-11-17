use {
    crate::rune_type::Rune,
    borsh::{BorshDeserialize, BorshSerialize},
    serde::{Deserialize, Deserializer, Serialize, Serializer},
    std::{
        fmt::{self, Display, Formatter},
        io::{Read, Write},
        str::FromStr,
    },
};

#[derive(Copy, Clone, Debug, PartialEq, Ord, PartialOrd, Eq, Default)]
pub struct SpacedRune {
    pub rune: Rune,
    pub spacers: u32,
}

impl SpacedRune {
    pub fn new(rune: Rune, spacers: u32) -> Self {
        Self { rune, spacers }
    }
}

impl FromStr for SpacedRune {
    type Err = SpacedRuneError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let mut rune = String::new();
        let mut spacers = 0u32;

        for c in s.chars() {
            match c {
                'A'..='Z' => rune.push(c),
                '.' | '•' => {
                    let flag = 1
                        << rune
                            .len()
                            .checked_sub(1)
                            .ok_or(SpacedRuneError::LeadingSpacer)?;
                    if spacers & flag != 0 {
                        return Err(SpacedRuneError::DoubleSpacer);
                    }
                    spacers |= flag;
                }
                _ => return Err(SpacedRuneError::Character(c)),
            }
        }

        if 32 - spacers.leading_zeros() >= rune.len().try_into().unwrap() {
            return Err(SpacedRuneError::TrailingSpacer);
        }

        Ok(SpacedRune {
            rune: rune.parse().map_err(SpacedRuneError::Rune)?,
            spacers,
        })
    }
}

impl Display for SpacedRune {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        let rune = self.rune.to_string();

        for (i, c) in rune.chars().enumerate() {
            write!(f, "{c}")?;

            if i < rune.len() - 1 && self.spacers & 1 << i != 0 {
                write!(f, "•")?;
            }
        }

        Ok(())
    }
}

impl Serialize for SpacedRune {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for SpacedRune {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = <String as Deserialize>::deserialize(deserializer)?;
        Self::from_str(&s).map_err(serde::de::Error::custom)
    }
}

impl BorshSerialize for SpacedRune {
    fn serialize<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        BorshSerialize::serialize(&self.rune.0, writer)?;
        BorshSerialize::serialize(&self.spacers, writer)
    }
}

impl BorshDeserialize for SpacedRune {
    fn deserialize_reader<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        let rune_value = u128::deserialize_reader(reader)?;
        let spacers = u32::deserialize_reader(reader)?;
        Ok(SpacedRune {
            rune: Rune(rune_value),
            spacers,
        })
    }
}

#[derive(Debug, PartialEq)]
pub enum SpacedRuneError {
    LeadingSpacer,
    TrailingSpacer,
    DoubleSpacer,
    Character(char),
    Rune(crate::rune_type::RuneError),
}

impl Display for SpacedRuneError {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            Self::Character(c) => write!(f, "invalid character `{c}`"),
            Self::DoubleSpacer => write!(f, "double spacer"),
            Self::LeadingSpacer => write!(f, "leading spacer"),
            Self::TrailingSpacer => write!(f, "trailing spacer"),
            Self::Rune(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for SpacedRuneError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display() {
        assert_eq!("A.B".parse::<SpacedRune>().unwrap().to_string(), "A•B");
        assert_eq!("A.B.C".parse::<SpacedRune>().unwrap().to_string(), "A•B•C");
        assert_eq!(
            SpacedRune {
                rune: Rune(0),
                spacers: 1
            }
            .to_string(),
            "A"
        );
    }

    #[test]
    fn from_str() {
        #[track_caller]
        fn case(s: &str, rune: &str, spacers: u32) {
            assert_eq!(
                s.parse::<SpacedRune>().unwrap(),
                SpacedRune {
                    rune: rune.parse().unwrap(),
                    spacers
                },
            );
        }

        assert_eq!(
            ".A".parse::<SpacedRune>().unwrap_err(),
            SpacedRuneError::LeadingSpacer,
        );

        assert_eq!(
            "A..B".parse::<SpacedRune>().unwrap_err(),
            SpacedRuneError::DoubleSpacer,
        );

        assert_eq!(
            "A.".parse::<SpacedRune>().unwrap_err(),
            SpacedRuneError::TrailingSpacer,
        );

        assert_eq!(
            "Ax".parse::<SpacedRune>().unwrap_err(),
            SpacedRuneError::Character('x')
        );

        case("A.B", "AB", 0b1);
        case("A.B.C", "ABC", 0b11);
        case("A•B", "AB", 0b1);
        case("A•B•C", "ABC", 0b11);
        case("A•BC", "ABC", 0b1);
    }

    #[test]
    fn serde() {
        let spaced_rune = SpacedRune {
            rune: Rune(26),
            spacers: 1,
        };
        let json = "\"A•A\"";
        assert_eq!(serde_json::to_string(&spaced_rune).unwrap(), json);
        assert_eq!(
            serde_json::from_str::<SpacedRune>(json).unwrap(),
            spaced_rune
        );
    }

    #[test]
    fn borsh_roundtrip() {
        let spaced_rune = SpacedRune {
            rune: Rune(42),
            spacers: 0b101,
        };
        let serialized = borsh::to_vec(&spaced_rune).unwrap();
        let deserialized: SpacedRune = borsh::from_slice(&serialized).unwrap();
        assert_eq!(spaced_rune, deserialized);
    }
}
