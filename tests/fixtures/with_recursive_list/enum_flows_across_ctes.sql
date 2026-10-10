-- An enum column carried from a recursive CTE into a later one orders and compares
-- by member position there.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE w (m mood);
INSERT INTO w VALUES ('sad'), ('happy');
CREATE TABLE tx (v TEXT);
-- @test
WITH RECURSIVE r(x) AS (SELECT m FROM w UNION ALL SELECT 'ok' FROM r WHERE x = 'sad'),
  q AS (SELECT x FROM r WHERE x > 'sad')
SELECT x::text FROM q ORDER BY x;
