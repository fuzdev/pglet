-- A NULL TIME sorts last under the default `ORDER BY t` (ASC NULLS LAST), and a
-- `DEFAULT` time literal resolves for the row that omits it — differential vs
-- Postgres.
-- @setup
CREATE TABLE e (id INT, t TIME DEFAULT '09:00:00');
INSERT INTO e (id, t) VALUES (1, '13:45:06'), (2, NULL), (3, '01:02:03');
INSERT INTO e (id) VALUES (4);

-- @test
SELECT id, t FROM e ORDER BY t;
