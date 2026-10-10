-- **The headline write of the qualifier-blind class, and a plain typo reaches it.** A
-- qualified correlated reference used to fall past the outer chain's identity tier into
-- a bare loop that never read the qualifier at all, so `zzz.v` resolved to the DELETE
-- target's own `v` and this statement was `DELETE 1` — row 1 gone — where 18.6 raises
-- `42P01 "zzz"` and both rows survive. No quoted column anywhere, no exotic FROM item:
-- the whole reproduction is a misspelled table alias.
--
-- The rule that closes it is the walk's, not the tier's: a frame that owns the qualifier
-- stops the walk, and a qualifier **no** frame owns leaves the chain entirely, so the
-- inner resolver reports the missing FROM-clause entry it always could. Controls
-- measured on both engines: `q2.yv = zzz.nosuch` is the same `42P01` (so this is not the
-- bare loop finding a name), and the ordinary `q2.yv = del_hw.v` answers.
--
-- **What this fixture cannot say is that the rows survived.** `@expect-error` compares
-- SQLSTATEs and nothing else, so it is green for a refusal raised *after* the writes.
-- `crates/pglet/tests/scope_relation_identity.rs` carries the row assertion for this
-- statement, and that pairing is the class's guard shape.
-- @setup
CREATE TABLE del_hw (id INT, v TEXT);
CREATE TABLE del_u (yv TEXT);
INSERT INTO del_hw VALUES (1, 'hit'), (2, 'miss');
INSERT INTO del_u VALUES ('hit');

-- @expect-error: missing FROM-clause entry for table "zzz" (42P01)
-- @test
DELETE FROM del_hw WHERE EXISTS (SELECT 1 FROM del_u WHERE del_u.yv = zzz.v);
