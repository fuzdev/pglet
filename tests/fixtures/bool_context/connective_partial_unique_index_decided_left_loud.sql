-- A partial unique index's membership of a second row the left operand admits: the row is a
-- member, so it collides — `23505`, where evaluating the division would be `22012`.
-- @profile-lock
-- @setup
CREATE TABLE su (id INT, s TEXT);
CREATE UNIQUE INDEX su_u ON su (s) WHERE (id = 1 OR 1/(id-1) = 0);
INSERT INTO su VALUES (1, 'a');
-- @expect-error: duplicate key value violates unique constraint "su_u"
-- @test
INSERT INTO su VALUES (1, 'a');
