-- The computed half of `enum/from_derived_table_passthrough_text_column_loud`: the
-- projection is an expression, so the descriptor decides nothing and the derivation is
-- the only reader — and it must answer *not an enum* here, not *undecidable*.
-- @expect-error: mood compared with a derived table's computed text column
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE w (id INT, m mood NOT NULL);
INSERT INTO w VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');
CREATE TABLE u (id INT, s TEXT NOT NULL);
INSERT INTO u VALUES (1, 'ok');

-- @test
SELECT w.id FROM w, (SELECT lower(s) AS z FROM u) v WHERE w.m = v.z;
