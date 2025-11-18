use {
    bitcoin::{address::FromScriptError, Address, Network, Script},
    clap::ValueEnum,
    serde::{Deserialize, Serialize},
    std::{
        fmt::{self, Display, Formatter},
        str::FromStr,
    },
    thiserror::Error,
    titan_types_core::Rune,
};

#[derive(Error, Debug, Clone, PartialEq)]
pub enum ChainError {
    #[error("invalid chain `{chain}`")]
    InvalidChain { chain: String },
}

#[derive(Default, ValueEnum, Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Chain {
    #[default]
    #[value(alias("main"))]
    Mainnet,
    #[value(alias("test"))]
    Testnet,
    Testnet4,
    Signet,
    Regtest,
}

impl Chain {
    pub(crate) fn network(self) -> Network {
        self.into()
    }

    pub(crate) fn first_rune_height(self) -> u32 {
        Rune::first_rune_height(self.into())
    }

    pub(crate) fn address_from_script(self, script: &Script) -> Result<Address, FromScriptError> {
        Address::from_script(script, self.network())
    }
}

impl From<Chain> for Network {
    fn from(chain: Chain) -> Network {
        match chain {
            Chain::Mainnet => Network::Bitcoin,
            Chain::Testnet => Network::Testnet,
            Chain::Signet => Network::Signet,
            Chain::Regtest => Network::Regtest,
            Chain::Testnet4 => Network::Testnet4,
        }
    }
}

impl Display for Chain {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::Mainnet => "mainnet",
                Self::Regtest => "regtest",
                Self::Signet => "signet",
                Self::Testnet => "testnet",
                Self::Testnet4 => "testnet4",
            }
        )
    }
}

impl FromStr for Chain {
    type Err = ChainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "mainnet" => Ok(Self::Mainnet),
            "regtest" => Ok(Self::Regtest),
            "signet" => Ok(Self::Signet),
            "testnet" => Ok(Self::Testnet),
            "testnet4" => Ok(Self::Testnet4),
            _ => Err(ChainError::InvalidChain {
                chain: s.to_string(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_str() {
        assert_eq!("mainnet".parse::<Chain>().unwrap(), Chain::Mainnet);
        assert_eq!("regtest".parse::<Chain>().unwrap(), Chain::Regtest);
        assert_eq!("signet".parse::<Chain>().unwrap(), Chain::Signet);
        assert_eq!("testnet".parse::<Chain>().unwrap(), Chain::Testnet);
        assert_eq!("testnet4".parse::<Chain>().unwrap(), Chain::Testnet4);
        assert_eq!(
            "foo".parse::<Chain>().unwrap_err().to_string(),
            "invalid chain `foo`"
        );
    }
}
