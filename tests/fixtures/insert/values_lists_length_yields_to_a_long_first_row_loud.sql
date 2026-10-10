-- The first row is longer than the table, and it is checked against the target list
-- before the second row is read: `INSERT has more expressions than target columns`, not the
-- second row's `VALUES lists must all be the same length`. Both are `42601`, so the message
-- is what this fixture locks.
-- @setup
CREATE TABLE vllf (i INT);

-- @test
-- @expect-error: INSERT has more expressions than target columns (42601)
-- @expect-message
INSERT INTO vllf VALUES (1, 2), (3);
