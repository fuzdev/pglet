-- An `INSERT … SELECT` whose source is a set operation assigns the set operation's
-- resolved column: `'2'` took `1`'s `integer`, so the row is stored.
-- @setup
CREATE TABLE ssu (i INT);
-- @test
INSERT INTO ssu (i) SELECT 1 UNION ALL SELECT '2' RETURNING i;
