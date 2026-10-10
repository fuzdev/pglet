-- The write the middle state would have opened, pinned at the polarity that matters: a
-- cast leaves the sub-plan column's identity undecidable, and a door that reads
-- *undecidable* as *fall back to the OID rule* removes two rows here. 18.6 refuses, and
-- so does this door — the `IN (SELECT …)` one takes the opposite side of the same trade
-- (`EDGES.md` row 114).
-- @expect-error: mood compared with a scalar subquery projecting a cast to text
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE w (id INT, m mood NOT NULL);
INSERT INTO w VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');
CREATE TABLE u (id INT, s TEXT NOT NULL);
INSERT INTO u VALUES (1, 'ok');

-- @test
DELETE FROM w WHERE m = (SELECT s::text FROM u);
