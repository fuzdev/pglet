-- The persisted doors' accept polarity: a member label in a `CHECK` and in a partial
-- index predicate is recorded on both engines, and the rows the index and the constraint
-- admit are the assertion.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, CHECK (greatest(m, 'sad') >= 'sad'));
CREATE INDEX tix ON t (id) WHERE m > 'sad';
INSERT INTO t VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');

-- @test
SELECT id, m::text AS label FROM t WHERE m > 'sad' ORDER BY id;
