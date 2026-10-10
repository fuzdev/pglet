-- The accept polarity over a **computed** sub-plan column, which is the shape the door's
-- accept fixtures did not have: every one of them projected a bare column, whose
-- `ColumnDesc::enum_type` carries the identity. A projected *expression*'s descriptor
-- derives none (`EDGES.md` row 107), and a reader that took that `None` for *not an
-- enum* refused every row here — a cast (which 18.6 resolves to the enum), `min`/`max`
-- (which return an argument's own value), a `GROUP BY` key, and the three value
-- constructors that unify their operands.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TYPE m3 AS ENUM ('sad', 'happy', 'ok');
CREATE TABLE w (id INT, m mood NOT NULL, y m3 NOT NULL, s TEXT NOT NULL);
INSERT INTO w VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'happy', 'ok'), (3, 'happy', 'sad', 'zzz');

-- @test
SELECT id,
       (m IN (SELECT 'ok'::mood)) AS cast_entry,
       (m IN (SELECT min(m) FROM w)) AS aggregate_entry,
       (m IN (SELECT m FROM w GROUP BY m)) AS group_key_entry,
       (m IN (SELECT COALESCE(m, 'ok') FROM w)) AS coalesce_entry,
       (m IN (SELECT CASE WHEN id > 0 THEN m END FROM w)) AS case_entry,
       (m IN (SELECT greatest(m, 'ok') FROM w)) AS greatest_entry
FROM w ORDER BY id;
