use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct Identifier([u8; 16]);

impl Identifier {
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    pub const fn bytes(self) -> [u8; 16] {
        self.0
    }

    pub fn to_hex(self) -> String {
        let mut text = String::with_capacity(32);
        for byte in self.0 {
            use std::fmt::Write;
            write!(text, "{byte:02x}").expect("writing to a string cannot fail");
        }
        text
    }
}

impl fmt::Display for Identifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_hex())
    }
}

impl FromStr for Identifier {
    type Err = &'static str;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text.len() != 32 {
            return Err("identifier must contain 32 hexadecimal digits");
        }
        let mut bytes = [0; 16];
        for (index, byte) in bytes.iter_mut().enumerate() {
            let start = index * 2;
            *byte = u8::from_str_radix(&text[start..start + 2], 16)
                .map_err(|_| "identifier contains a non-hexadecimal digit")?;
        }
        Ok(Self(bytes))
    }
}

macro_rules! identifier_type {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
        pub struct $name(pub Identifier);

        impl $name {
            pub const fn from_bytes(bytes: [u8; 16]) -> Self {
                Self(Identifier::from_bytes(bytes))
            }

            pub const fn bytes(self) -> [u8; 16] {
                self.0.bytes()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl FromStr for $name {
            type Err = &'static str;

            fn from_str(text: &str) -> Result<Self, Self::Err> {
                Identifier::from_str(text).map(Self)
            }
        }
    };
}

identifier_type!(CommandId);
identifier_type!(ParticipantId);
identifier_type!(ShareId);
identifier_type!(StrokeId);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hexadecimal_identifier_round_trips() {
        let identifier = Identifier::from_bytes([0x5a; 16]);
        assert_eq!(identifier.to_hex().parse(), Ok(identifier));
    }

    #[test]
    fn identifier_types_cannot_be_mixed_implicitly() {
        let participant = ParticipantId::from_bytes([1; 16]);
        let stroke = StrokeId::from_bytes([1; 16]);
        assert_eq!(participant.bytes(), stroke.bytes());
    }
}
