-- `unnest`'s result type **is** its operand's element type, so it passes the array node's
-- enum through unchanged — the inverse of the `ARRAY[…]` arm and the only spelling in the
-- corpus that reaches it.
--
-- The elements are scalar subqueries rather than casts on purpose: a cast's enum identity
-- is erased where the parser reads the type name (`EDGES.md` row 104), so an
-- `ARRAY['happy'::mood, …]` here would carry no identity for any walk to find.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'happy'), (2, 'sad');

-- @test
SELECT x::text AS label FROM (SELECT unnest(ARRAY[(SELECT m FROM t WHERE id = 1), (SELECT m FROM t WHERE id = 2)]) AS x) q ORDER BY x;
