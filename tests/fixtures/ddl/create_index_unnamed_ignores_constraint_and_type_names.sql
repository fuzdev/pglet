-- The accept side of the three above: what the name steps around is the relation
-- namespace **alone**. A `CHECK` constraint and an enum type each already carry the
-- would-be name, and the index takes it anyway (`ChooseRelationName(…, isconstraint =
-- false)`; `index_create` asks nothing of `pg_type`). A chooser that also read
-- constraint or type names would answer `ciu14_a_idx1` / `ciu14_b_idx1`.
-- @setup
CREATE TYPE ciu14_b_idx AS ENUM ('x');
CREATE TABLE ciu14 (a INT, b INT, CONSTRAINT ciu14_a_idx CHECK (a > 0));
CREATE INDEX ON ciu14 (a);
CREATE INDEX ON ciu14 (b);
-- @test
SELECT indexname FROM pg_indexes
WHERE schemaname = 'public' AND tablename = 'ciu14' ORDER BY indexname;
