-- A derived table's column is a **projection**, and a computed one carries no
-- `ColumnDesc::enum_type` (`EDGES.md` row 107) — so a gate that reads the descriptor
-- answers *not an enum* for `v.z` and refuses every comparison below, which 18.6
-- answers. The identity is derived from the projection instead, at the one site that
-- builds such a descriptor.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE w (id INT, m mood NOT NULL, s TEXT NOT NULL);
INSERT INTO w VALUES (1, 'sad', 'ok'), (2, 'ok', 'ok'), (3, 'happy', 'zzz');
CREATE TABLE u (id INT, m mood NOT NULL);
INSERT INTO u VALUES (1, 'ok');

-- @test
SELECT w.id,
       (w.m = v.z) AS eq,
       (w.m > v.z) AS gt,
       (w.m BETWEEN v.z AND 'happy') AS between_,
       COALESCE(w.m, v.z)::text AS coalesced,
       (w.m = ANY(ARRAY[v.z])) AS any_array,
       greatest(w.m, v.z)::text AS greatest_
FROM w, (SELECT min(m) AS z FROM u) v ORDER BY w.id;
