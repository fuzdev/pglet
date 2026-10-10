-- 18.6's rewriter rebuilds a `SET` list in column order, so the right-hand sides are
-- evaluated in that order: the first column's `22P02` answers although its assignment is
-- written second, ahead of the second column's `22012`. pglet answered `22012`.
-- @setup
CREATE TABLE soco (a INT, b INT);
INSERT INTO soco VALUES (1, 0);

-- @expect-error: invalid input syntax for type integer (22P02)
-- @test
UPDATE soco SET b = a / b, a = (a::text || 'x')::int;
