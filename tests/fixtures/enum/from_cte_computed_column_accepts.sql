-- The CTE spelling of `enum/from_derived_table_computed_column_accepts`: a `WITH`
-- reference registers its columns from the same descriptors, so the derivation has to
-- reach it by the same route rather than by a second one.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE w (id INT, m mood NOT NULL, s TEXT NOT NULL);
INSERT INTO w VALUES (1, 'sad', 'ok'), (2, 'ok', 'ok'), (3, 'happy', 'zzz');
CREATE TABLE u (id INT, m mood NOT NULL);
INSERT INTO u VALUES (1, 'ok');

-- @test
WITH c AS (SELECT min(m) AS z FROM u)
SELECT w.id, (w.m > c.z) AS gt, (w.m = c.z) AS eq, COALESCE(w.m, c.z)::text AS coalesced
FROM w, c ORDER BY w.id;
