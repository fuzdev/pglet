-- A string's order is the database collation's on 18.6 and the bytes' here, so an
-- ordering comparison of strings is not trusted to decide a branch: `'B' < 'a'` is true
-- by bytes and false under the oracle's collation, where the `sum` therefore runs.
-- Deciding by the byte order would answer `t`.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @expect-error: division by zero
-- @test
SELECT ('B' < 'a') OR sum(1/b) > 0 FROM u;
