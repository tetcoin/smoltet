//! Radix-16 Merkle-Patricia trie.
// TODO: write docs

use alloc::collections::BTreeMap;
use core::convert::TryFrom as _;
use hashbrown::{hash_map::Entry, HashMap};
use parity_scale_codec::Encode as _;

/// Radix-16 Merkle-Patricia trie.
pub struct Trie {
    entries: BTreeMap<Vec<u8>, Vec<u8>>,
}

impl Trie {
    /// Builds a new empty [`Trie`].
    pub fn new() -> Trie {
        Trie {
            entries: BTreeMap::new(),
        }
    }

    /// Inserts a new entry in the trie.
    pub fn insert(&mut self, key: impl Into<Vec<u8>>, value: impl Into<Vec<u8>>) {
        self.entries.insert(key.into(), value.into());
    }

    /// Returns true if the `Trie` is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Removes all the elements from the trie.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Removes from the trie all the keys that start with `prefix`, including `prefix` itself.
    pub fn remove_prefix(&mut self, prefix: &[u8]) {
        if prefix.is_empty() {
            self.clear();
            return;
        }

        let to_remove = self
            .entries
            .range(prefix.to_vec()..) // TODO: the necessity of this to_vec() is weird
            .take_while(|(k, _)| k.starts_with(prefix))
            .map(|(k, _)| k.clone())
            .collect::<Vec<_>>();
        for to_remove in to_remove {
            self.entries.remove(&to_remove);
        }
    }

    /// Calculates the Merkle value of the root node.
    pub fn root_merkle_value(&self) -> [u8; 32] {
        self.merkle_value(&[], None)
    }

    /// Calculates the Merkle value of the node with the given key.
    pub fn merkle_value(&self, key: &[u8], key_extra_nibble: Option<u8>) -> [u8; 32] {
        let node_value = self.node_value(key, key_extra_nibble);

        if (key.is_empty() && key_extra_nibble.is_none()) || node_value.len() >= 32 {
            let blake2_hash = blake2_rfc::blake2b::blake2b(32, &[], &node_value);
            let mut out = [0; 32];
            out.copy_from_slice(blake2_hash.as_bytes());
            out
        } else {
            debug_assert!(node_value.len() < 32);
            let mut out = [0; 32];
            // TODO: specs mention that the return value is always 32bits, but are unclear how to
            // extend a less than 32bits value to 32bits
            out[(32 - node_value.len())..].copy_from_slice(&node_value);
            out
        }
    }

    fn node_value(&self, key: &[u8], key_extra_nibble: Option<u8>) -> Vec<u8> {
        let partial_key_nibbles = self.node_partial_key_nibbles(key, key_extra_nibble);

        let partial_key = if partial_key_nibbles.len() % 2 == 0 {
            let mut pk = Vec::with_capacity(partial_key_nibbles.len() / 2);
            for chunk in partial_key_nibbles.chunks(2) {
                pk.push((chunk[0] << 4) | chunk[1]);
            }
            pk
        } else {
            let mut pk = Vec::with_capacity(1 + partial_key_nibbles.len() / 2);
            // What a weird encoding.
            pk.push(partial_key_nibbles[0]);
            for chunk in partial_key_nibbles[1..].chunks(2) {
                pk.push((chunk[0] << 4) | chunk[1]);
            }
            pk
        };
        
        let mut out = self.node_header(key, key_extra_nibble);
        out.extend(partial_key);
        out.extend(self.node_subvalue(key, key_extra_nibble));
        out
    }

    fn node_header(&self, key: &[u8], key_extra_nibble: Option<u8>) -> Vec<u8> {
        let two_msb: u8 = {
            let has_stored_value = key_extra_nibble.is_none() && self.entries.contains_key(key);
            let has_children = self.node_has_children(key, key_extra_nibble);
            match (has_stored_value, has_children) {
                (false, false) => 0b00, // TODO: is that exact? specs say "Special case"?!?!
                (true, false) => 0b01,
                (false, true) => 0b10,
                (true, true) => 0b11,
            }
        };

        let mut pk_len = self.node_partial_key_nibbles(key, key_extra_nibble).len();

        if pk_len >= 63 {
            pk_len -= 63;
            let mut out = vec![(two_msb << 6) + 63];
            while pk_len > 255 {
                pk_len -= 255;
                out.push(255);
            }
            out.push(u8::try_from(pk_len).unwrap());
            out

        } else {
            vec![(two_msb << 6) + u8::try_from(pk_len).unwrap()]
        }
    }

    fn node_partial_key_nibbles(&self, key: &[u8], key_extra_nibble: Option<u8>) -> Vec<u8> {
        Vec::new()  // TODO: FIXME: stub
    }

    fn node_subvalue(&self, key: &[u8], key_extra_nibble: Option<u8>) -> Vec<u8> {
        let encoded_stored_value = if key_extra_nibble.is_none() {
            self.entries.get(key).cloned().unwrap_or(Vec::new())
        } else {
            Vec::new()
        }
        .encode();

        let children_bitmap = self.node_children_bitmap(key, key_extra_nibble);
        if children_bitmap == 0 {
            return encoded_stored_value;
        }

        let mut out = children_bitmap.to_le_bytes().to_vec(); // TODO: LE? specs don't say anything, wtf

        if let Some(extra) = key_extra_nibble {
            for extra2 in 0..16 {
                let mut subkey = key.to_vec();
                subkey.push((extra << 4) | extra2);
                let child_merkle_value = self.merkle_value(&subkey, None);
                out.extend(child_merkle_value.encode());
            }
        } else {
            for extra in 0..16 {
                let child_merkle_value = self.merkle_value(key, Some(extra));
                out.extend(child_merkle_value.encode());
            }
        }

        out.extend(encoded_stored_value);
        out
    }

    fn node_children_bitmap(&self, key: &[u8], key_extra_nibble: Option<u8>) -> u16 {
        let mut out = 0u16;

        if let Some(key_extra_nibble) = key_extra_nibble {
            for n in 0..16 {
                let mut subkey = key.to_vec();
                subkey.push((key_extra_nibble << 4) | n);
                if self.node_has_children(&subkey, None) {
                    out |= 1 << (15 - n);
                }
            }
        } else {
            for n in 0..16 {
                if self.node_has_children(key, Some(n)) {
                    out |= 1 << (15 - n);
                }
            }
        }

        out
    }

    fn node_has_children(&self, key: &[u8], key_extra_nibble: Option<u8>) -> bool {
        let mut start = key.to_vec();
        let mut end = key.to_vec();
        start.push(0);
        end.push(255);
        self.entries.range(start..=end).next().is_some()
    }
}