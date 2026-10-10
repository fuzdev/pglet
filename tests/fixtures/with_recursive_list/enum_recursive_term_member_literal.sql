-- Control: a member label converts, and orders by member position.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE w (m mood);
INSERT INTO w VALUES ('sad'), ('happy');
CREATE TABLE tx (v TEXT);
-- @test
WITH RECURSIVE r(x) AS (SELECT m FROM w UNION ALL SELECT 'ok' FROM r WHERE x = 'sad')
SELECT x::text FROM r ORDER BY x;
