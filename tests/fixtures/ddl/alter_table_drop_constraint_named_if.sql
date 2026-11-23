-- `IF` is an unreserved keyword, so it names a constraint as well: `DROP CONSTRAINT if`
-- names a constraint called `if`, and `DROP CONSTRAINT IF EXISTS if` skips it when the
-- table has none.
-- @setup
CREATE TABLE dcif (a INT CONSTRAINT "if" CHECK (a > 0), b INT);

-- @test
ALTER TABLE dcif DROP CONSTRAINT if;
ALTER TABLE dcif DROP CONSTRAINT IF EXISTS if;
SELECT count(*) AS checks FROM pg_constraint WHERE conname = 'if';
