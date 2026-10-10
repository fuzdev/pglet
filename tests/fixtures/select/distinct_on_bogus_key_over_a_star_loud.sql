-- @setup
CREATE TABLE dbs (a INT, b INT, t TEXT);
INSERT INTO dbs VALUES (1, 5, 'x'), (2, 7, 'y');

-- The refusal polarity of the two fixtures beside this one: a bare `*` denotes every
-- column the scope carries, so a key naming one it does not is an ordinary unbound
-- reference — `42703`, not the `0A000` that used to answer every key over a `*`
-- projection whether or not it named anything.
-- @expect-error: a bogus DISTINCT ON key over a star projection is 42703
-- @test
SELECT DISTINCT ON (nosuch) * FROM dbs ORDER BY nosuch;
