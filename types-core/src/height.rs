use {
    bitcoin::blockdata::constants::DIFFCHANGE_INTERVAL,
    std::{
        fmt::{self, Display},
        ops::{Add, Sub},
        str::FromStr,
    },
};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Height(pub u32);

impl Height {
    pub fn n(self) -> u32 {
        self.0
    }

    pub fn period_offset(self) -> u32 {
        self.0 % DIFFCHANGE_INTERVAL
    }
}

impl Add<u32> for Height {
    type Output = Self;

    fn add(self, other: u32) -> Height {
        Self(self.0 + other)
    }
}

impl Sub<u32> for Height {
    type Output = Self;

    fn sub(self, other: u32) -> Height {
        Self(self.0 - other)
    }
}

impl PartialEq<u32> for Height {
    fn eq(&self, other: &u32) -> bool {
        self.0 == *other
    }
}

impl Display for Height {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for Height {
    type Err = <u32 as FromStr>::Err;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse::<u32>().map(Height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn n() {
        assert_eq!(Height(0).n(), 0);
        assert_eq!(Height(1).n(), 1);
    }

    #[test]
    fn add() {
        assert_eq!(Height(0) + 1, 1);
        assert_eq!(Height(1) + 100, 101);
    }

    #[test]
    fn sub() {
        assert_eq!(Height(1) - 1, 0);
        assert_eq!(Height(100) - 50, 50);
    }

    #[test]
    fn eq() {
        assert_eq!(Height(0), 0);
        assert_eq!(Height(100), 100);
    }

    #[test]
    fn from_str() {
        assert_eq!("0".parse::<Height>().unwrap(), 0);
        assert!("foo".parse::<Height>().is_err());
    }

    #[test]
    fn period_offset() {
        assert_eq!(Height(0).period_offset(), 0);
        assert_eq!(Height(1).period_offset(), 1);
        assert_eq!(Height(DIFFCHANGE_INTERVAL - 1).period_offset(), DIFFCHANGE_INTERVAL - 1);
        assert_eq!(Height(DIFFCHANGE_INTERVAL).period_offset(), 0);
        assert_eq!(Height(DIFFCHANGE_INTERVAL + 1).period_offset(), 1);
    }
}

