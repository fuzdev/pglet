-- The cast side of EDGES.md row 105: a non-member compared against an enum column ranked
-- after every member, so `m < 'q'::mood` counted every row.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT);
INSERT INTO t VALUES (1, 'sad', 'ok'), (2, 'ok', 'ok');

-- @test
-- @expect-error: q is not a member of mood
-- @expect-message
SELECT count(*) AS n FROM t WHERE m < 'q'::mood;
