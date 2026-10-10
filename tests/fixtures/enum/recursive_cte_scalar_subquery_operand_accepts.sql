-- The **loud** half of the same rebuild: with no identity on the recursive CTE's column,
-- comparing it against a real enum column was `42883 operator does not exist: mood > text`
-- — `EDGES.md` row 114's class at a door its accept list names separately from a plain
-- CTE. 18.6 answers, and so does pglet now.
--
-- This one is a **regression** guard rather than a conformance repair: in a build
-- before this row's gates existed, the same statement answered — and answered `3`, the
-- enum *column* operand supplying the order the CTE column could not. Measured. The
-- ordering fixtures beside it are the pre-existing half.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'ok'), (2, 'sad'), (3, 'happy');

-- @test
WITH RECURSIVE r(z) AS (
  SELECT m FROM t WHERE id = 1
  UNION ALL
  SELECT z FROM r WHERE false
)
SELECT id FROM t WHERE m > (SELECT z FROM r) ORDER BY id;
