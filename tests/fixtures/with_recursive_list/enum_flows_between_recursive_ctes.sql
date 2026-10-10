-- An enum column carried from one recursive CTE into the seed of a later one.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE w (m mood);
INSERT INTO w VALUES ('sad'), ('happy');
CREATE TABLE tx (v TEXT);
-- @test
WITH RECURSIVE r(x) AS (SELECT m FROM w UNION ALL SELECT 'ok' FROM r WHERE x = 'sad'),
  q(y) AS (SELECT x FROM r UNION SELECT 'happy' FROM q WHERE y = 'ok')
SELECT y::text, y > 'ok' AS above FROM q ORDER BY y;
