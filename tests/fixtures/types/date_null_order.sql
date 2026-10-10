-- A NULL DATE sorts last under the default `ORDER BY d` (ASC NULLS LAST), and a
-- `DEFAULT` date literal resolves — differential vs Postgres.
-- @setup
CREATE TABLE e (id INT, d DATE DEFAULT '2020-01-01');
INSERT INTO e (id, d) VALUES (1, '2024-06-15'), (2, NULL), (3, '2024-01-02');
INSERT INTO e (id) VALUES (4);

-- @test
SELECT id, d FROM e ORDER BY d;
