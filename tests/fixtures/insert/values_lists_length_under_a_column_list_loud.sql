-- Under a column list the rows' lengths are compared with each other first, as they are
-- without one: the short second row is `VALUES lists must all be the same length`, not the
-- column list's `INSERT has more target columns than expressions`.
-- @setup
CREATE TABLE vlcl (a INT, b INT);

-- @test
-- @expect-error: VALUES lists must all be the same length (42601)
-- @expect-message
INSERT INTO vlcl (a, b) VALUES (1, 2), (3);
