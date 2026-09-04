//! Small, checked wire format for the A1 control protocol.

const MAGIC: &[u8; 4] = b"NA1!";
const VERSION: u8 = 1;
const TYPE_HELLO: u8 = 1;
const TYPE_LSA: u8 = 2;

/// A1 documents at most 15 switches. This larger bound leaves room for
/// hand-written tests while keeping hostile/corrupt payload work bounded.
pub const MAX_LIST_ITEMS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hello {
    pub sender: u32,
    pub sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lsa {
    pub origin: u32,
    pub sequence: u64,
    pub neighbors: Vec<u32>,
    pub customers: Vec<u32>,
}

impl Lsa {
    pub fn new(
        origin: u32,
        sequence: u64,
        mut neighbors: Vec<u32>,
        mut customers: Vec<u32>,
    ) -> Self {
        neighbors.retain(|neighbor| *neighbor != origin);
        neighbors.sort_unstable();
        neighbors.dedup();
        customers.sort_unstable();
        customers.dedup();
        Self {
            origin,
            sequence,
            neighbors,
            customers,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlMessage {
    Hello(Hello),
    Lsa(Lsa),
}

fn put_header(out: &mut Vec<u8>, kind: u8) {
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.push(kind);
}

pub fn encode_hello(sender: u32, sequence: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(18);
    put_header(&mut out, TYPE_HELLO);
    out.extend_from_slice(&sender.to_le_bytes());
    out.extend_from_slice(&sequence.to_le_bytes());
    out
}

pub fn encode_lsa(lsa: &Lsa) -> Option<Vec<u8>> {
    if lsa.neighbors.len() > MAX_LIST_ITEMS || lsa.customers.len() > MAX_LIST_ITEMS {
        return None;
    }
    let neighbor_count = u16::try_from(lsa.neighbors.len()).ok()?;
    let customer_count = u16::try_from(lsa.customers.len()).ok()?;
    let capacity = 6 + 4 + 8 + 2 + 4 * lsa.neighbors.len() + 2 + 4 * lsa.customers.len();
    let mut out = Vec::with_capacity(capacity);
    put_header(&mut out, TYPE_LSA);
    out.extend_from_slice(&lsa.origin.to_le_bytes());
    out.extend_from_slice(&lsa.sequence.to_le_bytes());
    out.extend_from_slice(&neighbor_count.to_le_bytes());
    for neighbor in &lsa.neighbors {
        out.extend_from_slice(&neighbor.to_le_bytes());
    }
    out.extend_from_slice(&customer_count.to_le_bytes());
    for customer in &lsa.customers {
        out.extend_from_slice(&customer.to_le_bytes());
    }
    Some(out)
}

struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn take(&mut self, length: usize) -> Option<&'a [u8]> {
        let end = self.offset.checked_add(length)?;
        let result = self.bytes.get(self.offset..end)?;
        self.offset = end;
        Some(result)
    }

    fn u8(&mut self) -> Option<u8> {
        Some(*self.take(1)?.first()?)
    }

    fn u16(&mut self) -> Option<u16> {
        let mut bytes = [0u8; 2];
        bytes.copy_from_slice(self.take(2)?);
        Some(u16::from_le_bytes(bytes))
    }

    fn u32(&mut self) -> Option<u32> {
        let mut bytes = [0u8; 4];
        bytes.copy_from_slice(self.take(4)?);
        Some(u32::from_le_bytes(bytes))
    }

    fn u64(&mut self) -> Option<u64> {
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(self.take(8)?);
        Some(u64::from_le_bytes(bytes))
    }

    fn finished(&self) -> bool {
        self.offset == self.bytes.len()
    }
}

pub fn decode(bytes: &[u8]) -> Option<ControlMessage> {
    let mut cursor = Cursor::new(bytes);
    if cursor.take(MAGIC.len())? != MAGIC {
        return None;
    }
    if cursor.u8()? != VERSION {
        return None;
    }
    match cursor.u8()? {
        TYPE_HELLO => {
            let hello = Hello {
                sender: cursor.u32()?,
                sequence: cursor.u64()?,
            };
            cursor.finished().then_some(ControlMessage::Hello(hello))
        }
        TYPE_LSA => {
            let origin = cursor.u32()?;
            let sequence = cursor.u64()?;
            let neighbor_count = usize::from(cursor.u16()?);
            if neighbor_count > MAX_LIST_ITEMS {
                return None;
            }
            let mut neighbors = Vec::with_capacity(neighbor_count);
            for _ in 0..neighbor_count {
                neighbors.push(cursor.u32()?);
            }
            let customer_count = usize::from(cursor.u16()?);
            if customer_count > MAX_LIST_ITEMS {
                return None;
            }
            let mut customers = Vec::with_capacity(customer_count);
            for _ in 0..customer_count {
                customers.push(cursor.u32()?);
            }
            if !cursor.finished() {
                return None;
            }
            Some(ControlMessage::Lsa(Lsa::new(
                origin, sequence, neighbors, customers,
            )))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_round_trip() {
        let encoded = encode_hello(42, 99);
        assert_eq!(
            decode(&encoded),
            Some(ControlMessage::Hello(Hello {
                sender: 42,
                sequence: 99,
            }))
        );
    }

    #[test]
    fn lsa_round_trip_is_canonical() {
        let lsa = Lsa::new(7, 11, vec![9, 2, 9, 7], vec![0x0a00_0201, 3, 3]);
        let encoded = encode_lsa(&lsa).expect("bounded LSA");
        assert_eq!(decode(&encoded), Some(ControlMessage::Lsa(lsa.clone())));
        assert_eq!(lsa.neighbors, vec![2, 9]);
        assert_eq!(lsa.customers, vec![3, 0x0a00_0201]);
    }

    #[test]
    fn malformed_messages_are_rejected() {
        let valid = encode_hello(1, 2);
        for length in 0..valid.len() {
            assert_eq!(decode(&valid[..length]), None, "accepted length {length}");
        }
        let mut wrong_magic = valid.clone();
        wrong_magic[0] ^= 1;
        assert_eq!(decode(&wrong_magic), None);

        let mut wrong_version = valid.clone();
        wrong_version[4] = 99;
        assert_eq!(decode(&wrong_version), None);

        let mut trailing = valid;
        trailing.push(0);
        assert_eq!(decode(&trailing), None);
    }

    #[test]
    fn oversized_declared_list_is_rejected_before_allocation() {
        let mut bytes = Vec::new();
        put_header(&mut bytes, TYPE_LSA);
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&1u64.to_le_bytes());
        bytes.extend_from_slice(&u16::MAX.to_le_bytes());
        assert_eq!(decode(&bytes), None);
    }
}
