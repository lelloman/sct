//! Pure capacity transition model shared with the durable ledger.
//! All quantities are physical bytes in one allocation; mirror demand is added
//! per copy. Callers persist a successful transition in a fenced transaction.
use crate::error::{Error, Result, require};
use sct_protocol::ErrorCode;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Balance {
    pub capacity: u64,
    pub reserved: u64,
    pub occupied: u64,
    /// Subset of occupied, including bytes whose deletion is uncertain.
    pub pending: u64,
}
impl Balance {
    pub fn validate(&self) -> Result<()> {
        require(
            self.pending <= self.occupied
                && self
                    .occupied
                    .checked_add(self.reserved)
                    .is_some_and(|v| v <= self.capacity),
            ErrorCode::DestinationCapacity,
        )
    }
    pub fn free(&self) -> Result<u64> {
        self.validate()?;
        Ok(self.capacity - self.occupied - self.reserved)
    }
    pub fn reserve(&mut self, bytes: u64) -> Result<()> {
        require(bytes <= self.free()?, ErrorCode::StagingExhausted)?;
        self.reserved += bytes;
        Ok(())
    }
    /// Reserve -> occupied BEFORE sending bytes, even if acknowledgment is lost.
    pub fn issue_write(&mut self, bytes: u64) -> Result<()> {
        require(bytes <= self.reserved, ErrorCode::InternalError)?;
        self.reserved -= bytes;
        self.occupied += bytes;
        self.validate()
    }
    pub fn retire(&mut self, bytes: u64) -> Result<()> {
        require(
            bytes <= self.occupied - self.pending,
            ErrorCode::InternalError,
        )?;
        self.pending += bytes;
        Ok(())
    }
    /// Only an exact-generation confirmed deletion may invoke this transition.
    pub fn confirm_delete(&mut self, bytes: u64) -> Result<()> {
        require(bytes <= self.pending, ErrorCode::InternalError)?;
        self.pending -= bytes;
        self.occupied -= bytes;
        Ok(())
    }
    pub fn release_unused(&mut self, bytes: u64) -> Result<()> {
        require(bytes <= self.reserved, ErrorCode::InternalError)?;
        self.reserved -= bytes;
        Ok(())
    }
}
/// Validate the entire destination vector before mutating any pool. SQL callers
/// additionally lock pools in sorted order, then persist all results atomically.
pub fn reserve_all(
    pools: &mut BTreeMap<String, Balance>,
    demand: &BTreeMap<String, u64>,
) -> Result<()> {
    for (pool, bytes) in demand {
        require(
            *bytes
                <= pools
                    .get(pool)
                    .ok_or(Error(ErrorCode::InvalidRequest))?
                    .free()?,
            ErrorCode::StagingExhausted,
        )?;
    }
    for (pool, bytes) in demand {
        pools.get_mut(pool).unwrap().reserve(*bytes)?;
    }
    Ok(())
}
/// Publication is a charge transfer, not physical capacity recovery. Outgoing
/// pinned/deleting data stays charged to staging while new bytes enter retained.
pub fn publish(
    staging: &mut Balance,
    retained: &mut Balance,
    incoming: u64,
    outgoing: u64,
) -> Result<()> {
    let mut s = *staging;
    let mut r = *retained;
    require(
        incoming <= s.occupied - s.pending && outgoing <= r.occupied - r.pending,
        ErrorCode::InternalError,
    )?;
    s.occupied -= incoming;
    s.occupied = s
        .occupied
        .checked_add(outgoing)
        .ok_or(Error(ErrorCode::DestinationCapacity))?;
    s.pending = s
        .pending
        .checked_add(outgoing)
        .ok_or(Error(ErrorCode::DestinationCapacity))?;
    r.occupied -= outgoing;
    r.occupied = r
        .occupied
        .checked_add(incoming)
        .ok_or(Error(ErrorCode::DestinationCapacity))?;
    s.validate()?;
    r.validate()?;
    *staging = s;
    *retained = r;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_capacity_from_ambiguous_io_or_retirement() {
        let mut b = Balance {
            capacity: 100,
            ..Balance::default()
        };
        b.reserve(80).unwrap();
        b.issue_write(50).unwrap();
        // Crash/lost acknowledgment changes no charge; abort only releases unused.
        b.release_unused(30).unwrap();
        b.retire(50).unwrap();
        assert_eq!(b.free().unwrap(), 50);
        let uncertain = b;
        assert_eq!(uncertain, b);
        b.confirm_delete(50).unwrap();
        assert_eq!(b.free().unwrap(), 100);
        assert!(b.confirm_delete(50).is_err());
    }
    #[test]
    fn mirrored_atomic_admission_and_failed_rollback() {
        let mut p = BTreeMap::from([
            (
                "a".into(),
                Balance {
                    capacity: 100,
                    ..Balance::default()
                },
            ),
            (
                "b".into(),
                Balance {
                    capacity: 10,
                    ..Balance::default()
                },
            ),
        ]);
        let before = p.clone();
        assert!(
            reserve_all(
                &mut p,
                &BTreeMap::from([("a".into(), 20), ("b".into(), 20)])
            )
            .is_err()
        );
        assert_eq!(p, before);
        // Two required copies in the SAME pool consume twice the unique bytes.
        reserve_all(&mut p, &BTreeMap::from([("a".into(), 80)])).unwrap();
        assert!(reserve_all(&mut p, &BTreeMap::from([("a".into(), 30)])).is_err());
    }
    #[test]
    fn strict_rotation_conserves_charge_and_pinned_history_can_block() {
        for incoming in 0..=100 {
            for outgoing in 0..=100 {
                let mut s = Balance {
                    capacity: 100,
                    occupied: incoming,
                    ..Balance::default()
                };
                let mut r = Balance {
                    capacity: 100,
                    occupied: 100,
                    ..Balance::default()
                };
                let before = (s, r);
                match publish(&mut s, &mut r, incoming, outgoing) {
                    Ok(()) => {
                        assert_eq!(
                            s.occupied + r.occupied,
                            before.0.occupied + before.1.occupied
                        );
                        assert_eq!(s.pending, outgoing);
                        s.validate().unwrap();
                        r.validate().unwrap();
                    }
                    Err(_) => assert_eq!((s, r), before),
                }
            }
        }
    }
}
