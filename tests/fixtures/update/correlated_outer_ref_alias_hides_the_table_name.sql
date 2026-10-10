-- The correlated half of the `eref` rule, and the write
-- `update/alias_hides_the_table_name_in_where.sql` says it does *not* cover. Once `ahc_hw`
-- is aliased `x`, the qualifier `ahc_hw` names nothing — 18.6 answers `42P01` with a HINT
-- naming `x` — but the reference here is inside a correlated `EXISTS`, so it reached the
-- outer chain rather than `SimpleColumns`, and the chain's bare loop answered off the
-- frame's own `v`. This statement was `UPDATE 1`, writing `99` over the `hit` row.
--
-- Both spellings of the rule now refuse from the same fact: the frame carries the
-- statement's *effective* name (the alias when one is written), so no frame owns
-- `ahc_hw` and the reference leaves the chain. The accept polarity is the same statement
-- with `x.v`, which answers on both engines — and it is the control that rejects the
-- obvious wrong fix of always carrying the table's own name.
--
-- Rows: `@expect-error` is green for a refusal raised after the writes, so the surviving
-- rows are asserted in `crates/pglet/tests/scope_relation_identity.rs`.
-- @setup
CREATE TABLE ahc_hw (id INT, v TEXT);
CREATE TABLE ahc_u (yv TEXT);
INSERT INTO ahc_hw VALUES (1, 'hit'), (2, 'miss');
INSERT INTO ahc_u VALUES ('hit');

-- @expect-error: invalid reference to FROM-clause entry for table "ahc_hw" (42P01)
-- @test
UPDATE ahc_hw x SET id = 99 WHERE EXISTS (SELECT 1 FROM ahc_u WHERE ahc_u.yv = ahc_hw.v);
