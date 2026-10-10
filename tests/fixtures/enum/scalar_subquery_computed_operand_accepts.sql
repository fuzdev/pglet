-- The operand is a **scalar subquery** projecting an expression. Its identity used to be
-- read off the sub-plan's `ColumnDesc::enum_type`, which a projected expression does not
-- carry (`EDGES.md` row 107); the gate took that `None` for *not an enum* and refused.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE w (id INT, m mood NOT NULL);
INSERT INTO w VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');
CREATE TABLE u (id INT, m mood NOT NULL);
INSERT INTO u VALUES (1, 'ok');

-- @test
SELECT id,
       (m > (SELECT min(m) FROM u)) AS gt,
       (m = (SELECT max(m) FROM u)) AS eq,
       (m BETWEEN (SELECT min(m) FROM u) AND 'happy') AS between_,
       COALESCE(m, (SELECT min(m) FROM u))::text AS coalesced,
       (m IN ((SELECT min(m) FROM u))) AS in_list
FROM w ORDER BY id;
