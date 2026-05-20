use crate::BitList;

macro_rules! impl_try_conv {
    ($typ:ident) => {
        impl TryFrom<&BitList> for $typ {
            type Error = ();
            fn try_from(value: &BitList) -> Result<Self, Self::Error> {
                if value.len() > $typ::BITS as _ {
                    return Err(());
                }
                Ok($typ::from_le_bytes(value.to_le_bytes()))
            }
        }
        impl TryFrom<BitList> for $typ {
            type Error = ();
            fn try_from(value: BitList) -> Result<Self, Self::Error> {
                if value.len() > $typ::BITS as _ {
                    return Err(());
                }
                Ok($typ::from_le_bytes(value.to_le_bytes()))
            }
        }
    };
}

macro_rules! impl_from {
    ($($typ:ident),*) => {
        $(impl From<$typ> for BitList {
            fn from(value: $typ) -> Self {
                Self::from_le_bytes(&value.to_le_bytes())
            }
        })*
    }
}
impl_from!(u8, i8, u16, i16, u32, i32, u64, i64, u128, i128, usize, isize);
impl From<bool> for BitList {
    fn from(value: bool) -> Self {
        Self::single(value)
    }
}

impl_try_conv!(u8);
impl_try_conv!(u16);
impl_try_conv!(u32);
impl_try_conv!(u64);
impl_try_conv!(usize);
impl_try_conv!(u128);
