-- The accept side of the reach, and the control that bounds it: the comparison gate reads
-- a computed operand's type **only where the pair names an enum**. Over two computed
-- `text` operands it reads neither, so these comparison pairs keep the conservatism
-- `EDGES.md` row 65 records — this answers here and there alike, and would be a false
-- refusal if the widening were unconditional. A branch set is widened where the set names
-- an enum, which is a different rule and not this fixture's.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'OK'), (2, 'ok', 'OK'), (3, 'happy', 'OK');

-- @test
SELECT id,
       lower(s) = upper(s) AS computed_pair,
       (s || '') = s AS concat_pair,
       length(s) > 1 AS int_pair
  FROM t ORDER BY id;
