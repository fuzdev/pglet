-- An `ARRAY[…]` **expression** over an enum column, compared against another. The array
-- node's enum is its elements' unified one, and the comparator reads that same list at
-- every element — so `ARRAY[m] < ARRAY['ok'::mood]` selects the members declared before
-- `ok`, where a label-byte comparison selects exactly the opposite row.
--
-- The arm this reaches was a `None` justified as *enum arrays have no column form here*,
-- which is true of a column (`ms mood[]` is refused) and says nothing about an
-- `ARRAY[…]` expression, which pglet builds and compares.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'happy'), (2, 'sad'), (3, 'ok');

-- @test
SELECT id FROM t WHERE ARRAY[m] < ARRAY['ok'::mood] ORDER BY id;
