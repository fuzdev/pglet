-- `ALTER TABLE … DROP COLUMN … CASCADE` parses (it was a 42601 syntax error) and
-- behaves like the bare form when the only dependents are auto-dropped anyway.
-- @setup
CREATE TABLE dccp (id INT PRIMARY KEY, a INT, b INT, CHECK (a > 0));
INSERT INTO dccp VALUES (1, 5, 6);

-- @profile-lock
-- @test
ALTER TABLE dccp DROP COLUMN a CASCADE;
INSERT INTO dccp VALUES (2, 7);
SELECT id, b FROM dccp ORDER BY id;
