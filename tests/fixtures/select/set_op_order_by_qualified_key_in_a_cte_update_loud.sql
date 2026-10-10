-- The `UPDATE` polarity of the write, over the qualified spelling — the one that was
-- silent at **every** row count, so seeding the arms would not have saved it. `UPDATE 2`
-- here, writing `99` over both rows, against 18.6's `42P01`.
--
-- Row survival (that nothing is written before the refusal) is asserted by
-- `crates/pglet/tests/scope_relation_identity.rs`'s
-- `a_set_op_sort_key_with_a_bogus_qualifier_does_not_update`.
-- @setup
CREATE TABLE sop_n9 (a INT);
INSERT INTO sop_n9 VALUES (1), (2);
CREATE TABLE sopa (a INT);
CREATE TABLE sopb (a INT);
INSERT INTO sopa VALUES (7);

-- @expect-error: missing FROM-clause entry for table "bogusrel" (42P01)
-- @test
WITH c AS (SELECT a FROM sopa UNION SELECT a FROM sopb ORDER BY bogusrel.a) UPDATE sop_n9 SET a = 99;
