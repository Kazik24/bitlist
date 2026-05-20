# bitlist

[![License](https://img.shields.io/badge/license-MIT-blue.svg)](
https://github.com/Kazik24/bitlist)
[![Crate](https://img.shields.io/crates/v/bitlist.svg)](
https://crates.io/crates/bitlist)
[![Documentation](https://docs.rs/bitlist/badge.svg)](
https://docs.rs/bitlist)

Word-sized bit list implementation with bigint functionality.

Main type of this crate is `BitList` - dynamic bitset storing up to 57 bits (on 64-bit target) inline without allocation, heap allocating for more bits.

Size of `BitList` is equal to `size_of::<usize>()`, type is also niche optmizated so `Option<BitList>` is same size as `BitList`.

The crate is under developement, it's missing documentation and examples. But is otherwise usefull and most of the methods do what you expect. Please also checkout `BitsIter` type as it is not only an iterator but also has methods to effeciently find bits in sparse lists.
