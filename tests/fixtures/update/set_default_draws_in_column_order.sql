-- Two `DEFAULT`s drawing on one sequence draw in column order, whatever order the list
-- is written in: `a` takes 3 and `b` 4. pglet answered `4 | 3`.
-- @setup
CREATE SEQUENCE sodo_seq;
CREATE TABLE sodo (a INT DEFAULT nextval('sodo_seq'), b INT DEFAULT nextval('sodo_seq'), c INT);
INSERT INTO sodo (c) VALUES (0);

-- @test
UPDATE sodo SET b = DEFAULT, a = DEFAULT RETURNING a, b;
