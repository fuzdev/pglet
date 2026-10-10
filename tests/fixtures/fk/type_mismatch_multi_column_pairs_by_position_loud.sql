-- Pairs are by position in the two written lists, not by the key's column order: over a key
-- `(x int, y text)` referenced as `(y, x)`, an `(int, text)` referencing pair is refused, and
-- the `(text, int)` one is accepted (`type_multi_column_pairs_by_position_accepted`).
-- @setup
CREATE TABLE ftp (x INT, y TEXT, PRIMARY KEY (x, y));

-- @expect-error: 42804 names a and y
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ftc (a INT, b TEXT, FOREIGN KEY (a, b) REFERENCES ftp (y, x));
