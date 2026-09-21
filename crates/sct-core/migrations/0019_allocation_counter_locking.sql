-- Capacity admission serializes on the pool. Ordinary ledger counters do not
-- change the pool's allocation budget and must not acquire that lock after
-- locking an allocation: publication locks pool -> allocations, so the reverse
-- order deadlocks otherwise-independent uploads sharing a staging allocation.
-- Per-allocation CHECK constraints still guard all counter changes.
DROP TRIGGER allocation_capacity ON allocations;
CREATE TRIGGER allocation_capacity
BEFORE INSERT OR UPDATE OF capacity,pool_id,root_id ON allocations
FOR EACH ROW EXECUTE FUNCTION check_allocation_capacity();
