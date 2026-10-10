-- The same conversion guards the write a data-modifying body makes from it.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE w (m mood);
INSERT INTO w VALUES ('sad'), ('happy');
CREATE TABLE tx (v TEXT);
-- @expect-error: non-member literal converted to the seed enum (22P02)
-- @test
WITH RECURSIVE r(x) AS (SELECT m FROM w UNION ALL SELECT 'zzz' FROM r WHERE false)
INSERT INTO tx SELECT x FROM r;
