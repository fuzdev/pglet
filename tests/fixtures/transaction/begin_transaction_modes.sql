-- @no-txn
-- `gram.y`'s `transaction_mode_list` — the tail `parse_begin` used to reject as
-- leftover input, so `BEGIN ISOLATION LEVEL …` (what a driver writing its isolation
-- out longhand sends) was a `42601` and no block opened at all.
--
-- The block does real work so the fixture asserts that it *opened and committed*
-- rather than that a statement merely parsed: a `BEGIN` that parsed and did nothing
-- leaves the row uninserted, and one that parsed and never committed leaves it
-- rolled back.
--
-- The isolation level is honoured coarsely: a `REPEATABLE READ` block keeps its
-- snapshot, and its first write is refused `40001` if the store's commit stamp moved past it
-- (`docs/storage.md` § Isolation Guarantees, `EDGES.md` row 232). Seeing that takes a
-- second connection, which a fixture cannot script, so this fixture asserts only that
-- the block opened and committed.
-- @setup
DROP TABLE IF EXISTS begin_modes CASCADE;
CREATE TABLE begin_modes (n INT);
BEGIN ISOLATION LEVEL REPEATABLE READ READ WRITE NOT DEFERRABLE;
INSERT INTO begin_modes VALUES (1);
COMMIT;

-- @test
SELECT n FROM begin_modes;
