-- The accept polarity of the two arms just added, and what says they gate the pair rather
-- than the construct: an `IN` list and a simple `CASE` label over **member** labels are
-- recorded on both engines, and the rows the constraint admits are the assertion.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL,
                CHECK (m IN ('ok', 'happy')),
                CHECK ((CASE m WHEN 'ok' THEN 1 ELSE 0 END) >= 0));
INSERT INTO t VALUES (1, 'ok'), (2, 'happy');

-- @test
SELECT id, m::text AS label FROM t ORDER BY id;
