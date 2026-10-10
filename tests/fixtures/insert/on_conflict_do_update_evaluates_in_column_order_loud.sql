-- A `DO UPDATE SET` list is rebuilt in column order as a plain `UPDATE`'s is, so the
-- first column's `22P02` answers although its assignment is written second, ahead of
-- the second column's `22012`. pglet answered `22012`.
-- @profile-lock
-- @setup
CREATE TABLE occo (id INT PRIMARY KEY, a INT, b INT);
INSERT INTO occo VALUES (1, 1, 0);

-- @expect-error: invalid input syntax for type integer (22P02)
-- @test
INSERT INTO occo VALUES (1, 0, 0)
ON CONFLICT (id) DO UPDATE SET b = occo.a / occo.b, a = (occo.a::text || 'x')::int;
