-- Case folding must leave the `_` wildcard semantics untouched.
-- @setup
CREATE TABLE t (id INT, s TEXT);
INSERT INTO t VALUES (1, 'aXc'), (2, 'ABC'), (3, 'ab'), (4, 'a_c');

-- @test
SELECT * FROM t WHERE s ILIKE 'a_c' ORDER BY id;
