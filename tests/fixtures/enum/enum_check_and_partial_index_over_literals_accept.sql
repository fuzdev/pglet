-- The DDL-time gates' accept side: a `CHECK` and a partial index over an enum column and
-- an unknown literal are both recorded on 18.6, and both decide by the **member** order. The
-- surviving rows are the assertion.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, CHECK (COALESCE(m, 'sad') >= 'ok'));
CREATE INDEX tix ON t (id) WHERE m > 'ok';
INSERT INTO t VALUES (1, 'ok'), (2, 'happy');

-- @test
SELECT id, m::text AS label FROM t ORDER BY id;
