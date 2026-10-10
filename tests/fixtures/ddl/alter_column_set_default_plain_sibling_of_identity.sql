-- The identity refusal is a property of the *target column*, not of its table: a plain
-- column sitting beside an identity one keeps its default, set and cleared. The near-miss
-- this pins is a gate asking "does this table have an identity column", which is the same
-- shape that answers a missing column's 42703 as 42601 — but only the missing-column
-- fixture catches that shape when the gate sits *above* the column lookup. Written at the
-- gate's real position it over-rejects here and nowhere else, and every `@expect-error`
-- fixture in the identity family stays green while it does.
-- @setup
CREATE TABLE acsps_t (id INT GENERATED ALWAYS AS IDENTITY, b INT);
ALTER TABLE acsps_t ALTER COLUMN b SET DEFAULT 4;
INSERT INTO acsps_t (b) VALUES (DEFAULT);
ALTER TABLE acsps_t ALTER COLUMN b DROP DEFAULT;
INSERT INTO acsps_t (b) VALUES (DEFAULT);
-- @test
SELECT id, b FROM acsps_t ORDER BY id;
