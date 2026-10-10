-- A single-column UNIQUE permits multiple NULLs (default NULLS DISTINCT), so two
-- NULL-keyed rows both insert.
-- @setup
CREATE TABLE uq_nulls (id INT, email TEXT UNIQUE);
INSERT INTO uq_nulls VALUES (1, NULL), (2, NULL), (3, 'x@y');

-- @test
SELECT id, email FROM uq_nulls ORDER BY id;
