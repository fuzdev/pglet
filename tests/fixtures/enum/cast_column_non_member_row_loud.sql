-- A column holding a label the enum lacks is refused at the row that carries it.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE e (id INT, s TEXT);
INSERT INTO e VALUES (1, 'ok'), (2, 'q');

-- @test
-- @expect-error: q is not a member of mood
-- @expect-message
SELECT id, s::mood::text AS v FROM e ORDER BY id;
